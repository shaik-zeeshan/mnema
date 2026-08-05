<script lang="ts">
  // Overview — the converged frame-04 bento (redesign slice 10). One
  // edge-to-edge scroll surface of borderless-fill tiles under the material
  // title bar: moments strip, digest, capture/storage, conversations,
  // context, subjects, and the Ask launcher (which only ever LAUNCHES Quick
  // Look — an answer never renders here). Engine off → frame 06's honest
  // state: the provider card plus the tiles that need no AI.
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { untrack } from "svelte";
  import Button from "$lib/components/Button.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import Tile from "$lib/insights/Tile.svelte";
  import DayStrip from "$lib/insights/DayStrip.svelte";
  import { pushToast } from "$lib/toast.svelte";
  import { humanizeError } from "$lib/format-error";
  import { captureControls, sourceSelection } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";
  import { formatElapsed, formatPillBytes } from "$lib/components/record-pill";
  import { conversationStore, relativeTime } from "$lib/insights/conversationStore.svelte";
  import { getMainSurfaceSetting, setMainSurfaceSetting } from "$lib/main-surface";
  import { openSettings } from "$lib/surface-windows";
  import {
    clockHM,
    conversationDurationLabel,
    coverageHero,
    coverageLabel,
    firstSentence,
    momentsShownCount,
    monthlyPaceLabel,
    retentionLabel,
    speakersLabel,
    subjectRows,
  } from "$lib/insights/overview-format";
  import type { AudioSegmentDto } from "$lib/types/app-infra";
  import type {
    AuthoredContext,
    Conclusion,
    DayConversation,
    DayMoment,
    RecordingSettings,
    UserContextDigest,
  } from "$lib/types/recording";

  let {
    engineOff,
    statusLoaded,
    onOpenTab,
    onOpenSubject,
  }: {
    /** Engine never set up (enabled && configured is false) — frame 06. */
    engineOff: boolean;
    /** False while the shell's status calls are still in flight. */
    statusLoaded: boolean;
    onOpenTab: (tab: "context" | "subjects" | "journal") => void;
    onOpenSubject: (subject: string) => void;
  } = $props();

  // ── Width tiers (frame 04's three renders / the 800×600 drop ladder) ──
  let narrow = $state(false); // < 940 — 2-col ladder
  let wide = $state(false); // ≥ 1360 — 6-col
  $effect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const narrowQ = window.matchMedia("(max-width: 939px)");
    const wideQ = window.matchMedia("(min-width: 1360px)");
    const apply = () => {
      narrow = narrowQ.matches;
      wide = wideQ.matches;
    };
    apply();
    narrowQ.addEventListener("change", apply);
    wideQ.addEventListener("change", apply);
    return () => {
      narrowQ.removeEventListener("change", apply);
      wideQ.removeEventListener("change", apply);
    };
  });

  // ── Today's range (local midnight, half-open [start, start+24h)) ──
  function todayStartMs(): number {
    const midnight = new Date();
    midnight.setHours(0, 0, 0, 0);
    return midnight.getTime();
  }
  let dayStartMs = $state(todayStartMs());
  const DAY_MS = 24 * 3600_000;

  // ── Data (null = loading; [] / absent = a real empty state) ──
  let moments = $state<DayMoment[] | null>(null);
  let conversations = $state<DayConversation[] | null>(null);
  let digest = $state<UserContextDigest | null>(null);
  let digestLoaded = $state(false);
  let facts = $state<AuthoredContext[] | null>(null);
  let conclusions = $state<Conclusion[] | null>(null);
  let bytesToday = $state<number | null>(null);
  let historyBytes = $state<number | null>(null);
  let coverageMs = $state<number | null>(null);
  let settings = $state<RecordingSettings | null>(null);
  let defaultSurface = $state<string | null>(null);

  async function loadDayViews(): Promise<void> {
    dayStartMs = todayStartMs();
    const args = { startMs: dayStartMs, endMs: dayStartMs + DAY_MS };
    try {
      const [m, c] = await Promise.all([
        invoke<DayMoment[]>("list_moments_for_day", args),
        invoke<DayConversation[]>("list_conversations_for_day", args),
      ]);
      moments = m;
      conversations = c;
    } catch (error) {
      moments ??= [];
      conversations ??= [];
      pushToast("danger", "Couldn't load today's moments", { detail: humanizeError(error) });
    }
  }

  async function loadDigest(): Promise<void> {
    try {
      digest = await invoke<UserContextDigest | null>("get_user_context_digest", {
        rangeKind: "day",
        startMs: dayStartMs,
        endMs: dayStartMs + DAY_MS,
      });
    } catch {
      // Best-effort like the shipping Overview: a digest that can't generate
      // (engine busy/unreachable) renders the quiet empty line, not a toast.
      digest = null;
    } finally {
      digestLoaded = true;
    }
  }

  async function loadContext(): Promise<void> {
    try {
      const [authored, concluded] = await Promise.all([
        invoke<AuthoredContext[]>("list_user_context_authored"),
        invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }),
      ]);
      facts = authored;
      conclusions = concluded;
    } catch (error) {
      facts ??= [];
      conclusions ??= [];
      pushToast("danger", "Couldn't load your context", { detail: humanizeError(error) });
    }
  }

  async function loadCaptureStats(): Promise<void> {
    const since = todayStartMs();
    try {
      const [today, total, coverage] = await Promise.all([
        invoke<number>("get_bytes_captured_today", { sinceUnixMs: since }),
        invoke<number>("get_bytes_captured_today", { sinceUnixMs: 0 }),
        invoke<number>("get_capture_coverage_ms", { sinceUnixMs: since }),
      ]);
      bytesToday = today;
      historyBytes = total;
      coverageMs = coverage;
    } catch {
      // Best-effort readout (the RecordPill pattern): keep the last values.
    }
  }

  async function loadSettings(): Promise<void> {
    try {
      settings = await invoke<RecordingSettings>("get_recording_settings");
    } catch {
      // Retention line simply stays absent.
    }
  }

  $effect(() => {
    void untrack(() => {
      void loadDayViews();
      void loadContext();
      void loadCaptureStats();
      void loadSettings();
      void conversationStore.ensureStarted();
      void getMainSurfaceSetting().then((s) => (defaultSurface = s));
    });

    const stats = setInterval(() => void loadCaptureStats(), 30_000);
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen("user_context_changed", () => {
      void loadDayViews();
      void loadContext();
      if (untrack(() => statusLoaded && !engineOff)) void loadDigest();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      clearInterval(stats);
      unlisten?.();
    };
  });

  // The digest is engine-written — only fetch once the shell confirmed the
  // engine is set up (frame 06 owns the off state; no doomed generate calls).
  let digestKicked = false;
  $effect(() => {
    if (!statusLoaded || engineOff || digestKicked) return;
    digestKicked = true;
    void loadDigest();
  });

  // ── Capture tile (live state straight from the shared seams) ──
  let nowMs = $state(Date.now());
  $effect(() => {
    if (!captureControls.running) return;
    nowMs = Date.now();
    const tick = setInterval(() => (nowMs = Date.now()), 1_000);
    return () => clearInterval(tick);
  });
  const sessionStartMs = $derived.by(() => {
    const sessions = captureSession.value?.sourceSessions;
    if (!sessions) return null;
    const starts = [sessions.screen, sessions.microphone, sessions.systemAudio]
      .filter((s) => s !== null)
      .map((s) => s.startedAtUnixMs);
    return starts.length > 0 ? Math.min(...starts) : null;
  });
  const elapsed = $derived(
    captureControls.running && sessionStartMs !== null
      ? formatElapsed(nowMs - sessionStartMs)
      : "",
  );
  const sourcesLabel = $derived.by(() => {
    const names = (
      [
        ["screen", "screen"],
        ["microphone", "mic"],
        ["systemAudio", "system audio"],
      ] as const
    )
      .filter(([key]) => sourceSelection.isSelected(key))
      .map(([, name]) => name);
    if (names.length === 3) return "all three sources";
    if (names.length === 0) return "no sources selected";
    return names.join(" + ");
  });

  // ── Derived tile content ──
  const shownMoments = $derived(
    (moments ?? []).slice(0, momentsShownCount(narrow, wide)),
  );
  const subjects = $derived(conclusions === null ? null : subjectRows(conclusions));
  const newestFact = $derived(facts && facts.length > 0 ? facts[0] : null);
  const paceLine = $derived(bytesToday === null ? null : monthlyPaceLabel(bytesToday));
  const askHistory = $derived(conversationStore.conversations.slice(0, 2));
  const headerMeta = $derived.by(() => {
    const parts: string[] = [];
    if (coverageMs !== null) parts.push(`${coverageLabel(coverageMs)} captured`);
    const n = conversations?.length ?? 0;
    if (n > 0) parts.push(`${n} ${n === 1 ? "conversation" : "conversations"}`);
    return parts.join(" · ");
  });
  const dayTitle = $derived(
    new Date(dayStartMs).toLocaleDateString(undefined, {
      weekday: "long",
      month: "long",
      day: "numeric",
    }),
  );

  // ── Handoffs ──

  // Moment → Main Timeline at that instant (the Quick Recall result path).
  async function openMoment(moment: DayMoment): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: moment.frameId,
        audioSegmentId: null,
        spanStartMs: null,
        alignedFrameId: null,
      });
    } catch (error) {
      pushToast("danger", "Couldn't open that moment in the Timeline", {
        detail: humanizeError(error),
      });
    }
  }

  // Conversation → Timeline with the AudioDrawer open at its start: resolve
  // the first audio segment overlapping the conversation, then hand off
  // exactly like an audio search result (kind:"audio" + span offset).
  async function openConversation(c: DayConversation): Promise<void> {
    try {
      const segments = await invoke<AudioSegmentDto[]>("list_audio_segments", {
        request: {
          capturedAtStart: new Date(c.startedAtMs).toISOString(),
          capturedAtEnd: new Date(c.displayEndedAtMs).toISOString(),
        },
      });
      const first = [...segments].sort(
        (a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt),
      )[0];
      if (!first) {
        pushToast("info", "The audio for this conversation is no longer available.");
        return;
      }
      await invoke("open_capture_result_in_main_window", {
        kind: "audio",
        frameId: null,
        audioSegmentId: first.id,
        spanStartMs: Math.max(0, c.startedAtMs - Date.parse(first.startedAt)),
        alignedFrameId: null,
      });
    } catch (error) {
      pushToast("danger", "Couldn't open that conversation", {
        detail: humanizeError(error),
      });
    }
  }

  // Ask launcher → Quick Look in Ask mode. Never renders an answer here.
  let askDraft = $state("");
  async function submitAsk(): Promise<void> {
    const question = askDraft.trim();
    if (question.length === 0) return;
    try {
      await invoke("open_quick_recall_ask", { question, conversationId: null });
      askDraft = "";
    } catch (error) {
      pushToast("danger", "Couldn't open Ask", { detail: humanizeError(error) });
    }
  }
  async function reopenAskConversation(conversationId: string): Promise<void> {
    try {
      await invoke("open_quick_recall_ask", { question: null, conversationId });
    } catch (error) {
      pushToast("danger", "Couldn't reopen that conversation", {
        detail: humanizeError(error),
      });
    }
  }

  async function pinSurfaceHere(): Promise<void> {
    try {
      await setMainSurfaceSetting("overview");
      defaultSurface = "overview";
      pushToast("success", "Mnema now opens on Overview");
    } catch (error) {
      pushToast("danger", "Couldn't change the default surface", {
        detail: humanizeError(error),
      });
    }
  }
</script>

<div class="bento">
  <div class="bento__col">
    <header class="bento__head">
      <h1 class="t-title">{dayTitle}</h1>
      <p class="bento__meta">{headerMeta}</p>
      {#if defaultSurface !== null && defaultSurface !== "overview" && !narrow}
        <span class="bento__pin">
          <Button variant="ghost" size="sm" onclick={() => void pinSurfaceHere()}>
            Open Mnema here
          </Button>
        </span>
      {/if}
    </header>

    {#if statusLoaded && engineOff}
      <!-- Frame 06 — engine not configured. One calm card says what is off
           and why; the local, non-AI tiles stay useful below it. -->
      <div class="tiles">
        <Tile class="bt-off">
          <div class="off">
            <span class="off__icon" aria-hidden="true">
              <svg viewBox="0 0 24 24" fill="currentColor">
                <path
                  d="M12 3l1.9 5.6L19.5 10l-5.6 1.9L12 17.5l-1.9-5.6L4.5 10l5.6-1.4L12 3z"
                />
              </svg>
            </span>
            <div class="off__txt">
              <h2 class="off__title">Overview runs on an AI provider you choose</h2>
              <p class="off__body">
                Summaries, subjects and Ask need a reasoning engine — local (Ollama,
                Llamafile) or your own cloud key. Nothing is captured differently and
                nothing leaves this Mac until you pick one.
              </p>
              <div class="off__actions">
                <Button variant="primary" onclick={() => void openSettings("intelligence")}>
                  Choose Provider…
                </Button>
              </div>
            </div>
          </div>
        </Tile>

        {#if moments !== null && moments.length > 0}
          <Tile class="bt-strip" media>
            <DayStrip moments={shownMoments} onOpen={(m) => void openMoment(m)} />
          </Tile>
        {/if}

        {@render captureTile()}
        {@render storageTile()}

        {#if conversations !== null && conversations.length > 0}
          {@render conversationsTile()}
        {/if}
      </div>
    {:else}
      <div class="tiles">
        {#if moments === null}
          <Tile class="bt-strip" media>
            <div class="strip-skel"><Skeleton height="100%" radius="0" /></div>
          </Tile>
        {:else if moments.length > 0}
          <Tile class="bt-strip" media>
            <DayStrip moments={shownMoments} onOpen={(m) => void openMoment(m)} />
          </Tile>
        {/if}

        <Tile
          class="bt-today"
          label="Today"
          more={digest ? `updated ${clockHM(digest.generatedAtMs)}` : undefined}
        >
          {#if !statusLoaded || !digestLoaded}
            <div class="skel-lines">
              <Skeleton variant="text" width="92%" />
              <Skeleton variant="text" width="84%" />
              <Skeleton variant="text" width="56%" muted />
            </div>
          {:else if digest}
            <p class="today__prose">
              {#if digest.headline}<b>{digest.headline}</b>{" "}{/if}
              {narrow ? firstSentence(digest.narrative) : digest.narrative}
            </p>
          {:else}
            <p class="tile-empty">
              Not enough activity yet today — the digest appears as your day fills in.
            </p>
          {/if}
        </Tile>

        {@render captureTile()}
        {@render storageTile()}
        {@render conversationsTile()}

        <Tile
          class="bt-context"
          label="Context"
          more={facts && facts.length > 0 ? `${facts.length} facts` : undefined}
        >
          {#if facts === null}
            <div class="skel-lines">
              <Skeleton variant="text" width="60%" />
              <Skeleton variant="text" width="80%" muted />
            </div>
          {:else}
            <div class="tile-row">
              <span class="row-strong">
                {facts.length === 1 ? "1 fact about you" : `${facts.length} facts about you`}
              </span>
            </div>
            {#if newestFact}
              <div class="tile-row">
                <span class="row-meta row-clip">Newest: “{newestFact.text}”</span>
              </div>
            {/if}
            <button type="button" class="tile-link" onclick={() => onOpenTab("context")}>
              <span>Review all</span>
              <span class="chev" aria-hidden="true">›</span>
            </button>
          {/if}
        </Tile>

        <Tile
          class="bt-subjects"
          label="Subjects"
          more={subjects && subjects.length > 0 ? `${subjects.length} active` : undefined}
        >
          {#if subjects === null}
            <div class="skel-lines">
              <Skeleton variant="text" width="70%" />
              <Skeleton variant="text" width="64%" muted />
            </div>
          {:else if subjects.length === 0}
            <p class="tile-empty">
              No subjects yet — beliefs form as the engine reads more of your days.
            </p>
          {:else}
            {#each subjects.slice(0, 2) as row (row.subject)}
              <button
                type="button"
                class="grow"
                onclick={() => onOpenSubject(row.subject)}
              >
                <span class="grow__txt">
                  <span class="grow__lbl">{row.subject}</span>
                  <span class="grow__sub">{row.statement}</span>
                </span>
                <span class="grow__val">
                  <span class="conv" aria-label={`conviction ${row.dots} of 5`}>
                    {#each Array.from({ length: 5 }, (_, i) => i) as i (i)}
                      <i class:on={i < row.dots}></i>
                    {/each}
                  </span>
                  <span class="chev" aria-hidden="true">›</span>
                </span>
              </button>
            {/each}
          {/if}
        </Tile>

        <Tile class="bt-ask" label="Ask" more={askHistory.length > 0 ? "history" : undefined}>
          {#each askHistory as c (c.conversationId)}
            <button
              type="button"
              class="grow grow--ask"
              onclick={() => void reopenAskConversation(c.conversationId)}
            >
              <span class="grow__txt">
                <span class="grow__lbl">{c.title || c.preview}</span>
                <span class="grow__sub">
                  {c.turnCount === 1 ? "1 turn" : `${c.turnCount} turns`}
                </span>
              </span>
              <span class="grow__val">
                <span class="row-stamp">{relativeTime(c.updatedAtMs)}</span>
                <span class="chev" aria-hidden="true">›</span>
              </span>
            </button>
          {/each}
          <form
            class="ask-row"
            onsubmit={(event) => {
              event.preventDefault();
              void submitAsk();
            }}
          >
            <svg
              class="ask-row__icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              aria-hidden="true"
            >
              <circle cx="11" cy="11" r="7" />
              <path d="m20 20-3.5-3.5" />
            </svg>
            <input
              class="input ask-row__input"
              type="text"
              placeholder="Ask about your day…"
              aria-label="Ask about your day (opens Quick Look)"
              bind:value={askDraft}
            />
            <kbd class="kbd">⏎</kbd>
          </form>
        </Tile>
      </div>
    {/if}
  </div>
</div>

{#snippet captureTile()}
  <Tile class="bt-capture" label="Capture">
    <div class="cap-hero">
      {#if coverageMs === null}
        <Skeleton width="72px" height="26px" radius="6px" />
      {:else}
        <!-- The screen's ONE --t-display use. -->
        <span class="t-display is-num cap-hero__n">{coverageHero(coverageMs)}</span>
        <span class="cap-hero__u">hours today</span>
      {/if}
    </div>
    <div class="tile-row">
      <i
        class="dot"
        class:dot--running={captureControls.statusModifier === "running"}
        class:dot--paused={captureControls.statusModifier === "paused"}
      ></i>
      <span class="row-ui">{captureControls.statusLabel}</span>
      {#if elapsed}<span class="row-stamp row-end">{elapsed}</span>{/if}
    </div>
    <div class="tile-row">
      <span class="row-meta">{captureControls.running ? sourcesLabel : "not recording"}</span>
    </div>
  </Tile>
{/snippet}

{#snippet storageTile()}
  <Tile class="bt-storage" label="Storage">
    {#if bytesToday === null}
      <div class="skel-lines">
        <Skeleton variant="text" width="56%" />
        <Skeleton variant="text" width="78%" muted />
      </div>
    {:else}
      <div class="tile-row">
        <span class="row-strong">{formatPillBytes(bytesToday) || "0 MB"} today</span>
      </div>
      <div class="tile-row">
        <span class="row-meta">
          {historyBytes === null ? "" : `${formatPillBytes(historyBytes) || "0 MB"} of history`}
          {settings ? ` · ${retentionLabel(settings.retentionPolicy)}` : ""}
        </span>
      </div>
      {#if paceLine}
        <div class="tile-row"><span class="row-faint">{paceLine}</span></div>
      {/if}
    {/if}
  </Tile>
{/snippet}

{#snippet conversationsTile()}
  <Tile
    class="bt-convs"
    label="Conversations"
    more={conversations && conversations.length > 0
      ? `${conversations.length} today`
      : undefined}
  >
    {#if conversations === null}
      <div class="skel-lines">
        <Skeleton variant="text" width="66%" />
        <Skeleton variant="text" width="52%" muted />
      </div>
    {:else if conversations.length === 0}
      <p class="tile-empty">
        No conversations yet today — audio with at least two minutes of speech shows
        up here.
      </p>
    {:else}
      {#each conversations as c (c.activityId)}
        <button type="button" class="grow" onclick={() => void openConversation(c)}>
          <span class="gthumb" aria-hidden="true">
            <svg viewBox="0 0 50 28" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <path d="M7 12v4M13 9v10M19 12v4M25 6v16M31 10v8M37 13v2M43 9v10" />
            </svg>
          </span>
          <span class="grow__txt">
            <span class="grow__lbl">{c.title}</span>
            <span class="grow__sub">
              {conversationDurationLabel(c)} · {speakersLabel(c.speakerCount)}
            </span>
          </span>
          <span class="grow__val">
            <span class="row-stamp conv-when">{clockHM(c.startedAtMs)}</span>
            <span class="row-stamp conv-dur">{conversationDurationLabel(c)}</span>
            <span class="chev" aria-hidden="true">›</span>
          </span>
        </button>
      {/each}
    {/if}
  </Tile>
{/snippet}

<style>
  /* ── Surface: one scroll region running edge-to-edge under the material
     title bar (Overview-only; the frozen Timeline stays opaque). ── */
  .bento {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .bento__col {
    padding: calc(var(--app-titlebar-height, 36px) + var(--s-16)) var(--pad-window)
      var(--s-48);
  }

  .bento__head {
    display: flex;
    align-items: baseline;
    gap: var(--s-12);
    margin-bottom: var(--gap-group);
    min-height: 28px;
  }

  .bento__head h1 {
    margin: 0;
  }

  .bento__meta {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bento__pin {
    margin-left: auto;
    flex: 0 0 auto;
  }

  /* ── The grid (4-col default / 2-col ladder / 6-col wide) ── */
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    grid-auto-flow: dense;
    gap: var(--grid-gutter);
  }

  .tiles > :global(.bt-strip) {
    grid-column: 1 / -1;
    --strip-h: 148px;
  }
  .tiles > :global(.bt-off) {
    grid-column: 1 / -1;
  }
  .tiles > :global(.bt-today),
  .tiles > :global(.bt-convs),
  .tiles > :global(.bt-context),
  .tiles > :global(.bt-subjects),
  .tiles > :global(.bt-ask) {
    grid-column: span 2;
  }
  .tiles > :global(.bt-capture),
  .tiles > :global(.bt-storage) {
    grid-column: span 1;
  }

  @media (min-width: 1360px) {
    .tiles {
      grid-template-columns: repeat(6, 1fr);
    }
    .tiles > :global(.bt-strip) {
      grid-column: span 4;
      --strip-h: 170px;
    }
    .tiles > :global(.bt-today) {
      grid-column: span 3;
    }
    .tiles > :global(.bt-context) {
      grid-column: span 1;
    }
    .tiles > :global(.bt-ask) {
      grid-column: span 4;
    }
  }

  /* 800×600 drop ladder (DECISIONS): hero STAYS; the storage line ("270 MB
     today") drops with its tile; speaker counts, ask history and the
     one-sentence digest are handled below / in script. */
  @media (max-width: 939px) {
    .tiles {
      grid-template-columns: repeat(2, 1fr);
    }
    .tiles > :global(.bt-strip) {
      --strip-h: 110px;
    }
    .tiles > :global(.bt-today),
    .tiles > :global(.bt-ask) {
      grid-column: span 2;
    }
    .tiles > :global(.bt-capture),
    .tiles > :global(.bt-convs),
    .tiles > :global(.bt-context),
    .tiles > :global(.bt-subjects) {
      grid-column: span 1;
    }
    .tiles > :global(.bt-storage) {
      display: none;
    }
    .grow--ask {
      display: none;
    }
    .grow .gthumb,
    .grow .grow__sub,
    .grow__val .conv-when {
      display: none;
    }
    .grow__val .conv-dur {
      display: inline;
    }
  }

  /* ── Shared row anatomy (13's group rows reused inside a tile) ── */
  .tile-row {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-height: var(--h-row);
  }

  .row-strong {
    font: var(--w-regular) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .row-ui {
    font: var(--w-regular) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text);
  }
  .row-meta {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .row-faint {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .row-clip {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-stamp {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .row-end {
    margin-left: auto;
  }

  .tile-empty {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
    /* Reserved space — an empty tile keeps roughly its filled height so a
       late-arriving row never reflows the grid (field__help pattern). */
    min-height: calc(var(--h-row) * 2);
    display: flex;
    align-items: center;
  }

  .skel-lines {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding: var(--s-4) 0;
    min-height: calc(var(--h-row) * 2);
    justify-content: center;
  }

  .strip-skel {
    height: var(--strip-h, 148px);
  }

  /* Full-bleed interactive rows (subjects / conversations / ask history). */
  .grow {
    position: relative;
    width: 100%;
    min-height: 40px;
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: var(--s-8) 0;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    border-radius: var(--r-sm);
  }
  .grow + .grow::before {
    content: "";
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: var(--hairline);
    background: var(--app-border);
  }
  .grow:hover {
    background: color-mix(in srgb, var(--app-surface) 78%, var(--app-surface-hover));
  }
  .grow:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .grow__txt {
    min-width: 0;
    flex: 1 1 auto;
  }
  .grow__lbl {
    display: block;
    font: var(--w-regular) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow__sub {
    display: block;
    margin-top: 2px;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow__val {
    margin-left: auto;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-8);
  }
  .conv-dur {
    display: none;
  }

  .chev {
    color: var(--app-text-faint);
    font-size: 13px;
    line-height: 1;
    flex: 0 0 auto;
  }

  .gthumb {
    width: 50px;
    height: 28px;
    border-radius: var(--r-sm);
    overflow: hidden;
    flex: 0 0 auto;
    background: #0b0b10;
    color: color-mix(in srgb, var(--app-accent) 72%, #ffffff);
    display: inline-flex;
  }
  .gthumb svg {
    width: 100%;
    height: 100%;
  }

  /* Conviction dots (frame 04's Subjects meter — dots, not bars). */
  .conv {
    display: inline-flex;
    gap: 3px;
  }
  .conv i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }
  .conv i.on {
    background: var(--app-accent);
  }

  /* ── Today (digest prose) ── */
  .today__prose {
    margin: 0;
    max-width: 70ch;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
  }
  .today__prose b {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }

  /* ── Capture ── */
  .cap-hero {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
    min-height: 28px;
  }
  .cap-hero__n {
    font-variant-numeric: tabular-nums;
  }
  .cap-hero__u {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-text-subtle);
    flex: 0 0 auto;
  }
  .dot--running {
    background: var(--app-record);
    animation: bento-pulse 2s var(--ease) infinite;
  }
  .dot--paused {
    background: var(--app-warn);
  }
  @keyframes bento-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot--running {
      animation: none;
    }
  }

  /* ── Context "Review all" ── */
  .tile-link {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    width: 100%;
    min-height: var(--h-row);
    padding: 0;
    border: 0;
    background: transparent;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
    cursor: pointer;
    text-align: left;
    border-radius: var(--r-sm);
  }
  .tile-link .chev {
    margin-left: auto;
  }
  .tile-link:hover {
    color: var(--app-text);
  }
  .tile-link:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  /* ── Ask launcher row ── */
  .ask-row {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding-top: var(--s-8);
  }
  .grow--ask + .ask-row {
    border-top: var(--hairline) solid var(--app-border);
  }
  .ask-row__icon {
    width: 15px;
    height: 15px;
    color: var(--app-text-subtle);
    flex: 0 0 auto;
  }
  .ask-row__input {
    flex: 1 1 auto;
    min-width: 0;
  }

  /* ── Engine-off card (frame 06) ── */
  .off {
    display: flex;
    align-items: flex-start;
    gap: var(--s-12);
    padding: var(--s-8) 0;
  }
  .off__icon {
    width: 20px;
    height: 20px;
    border-radius: 5px;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-top: 2px;
    background: var(--app-accent);
    color: var(--app-accent-contrast, #08130c);
  }
  .off__icon svg {
    width: 11px;
    height: 11px;
  }
  .off__txt {
    min-width: 0;
  }
  .off__title {
    margin: 0;
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  .off__body {
    margin: var(--s-6) 0 0;
    max-width: 70ch;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-muted);
  }
  .off__actions {
    display: inline-flex;
    gap: var(--s-8);
    margin-top: var(--s-12);
  }
</style>
