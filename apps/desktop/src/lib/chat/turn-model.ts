// The Chat turn render model: the backend-owned `TurnView` (ADR 0031, issue
// #110) plus a few UI-only fields, and the pure reducer that applies streamed
// `TurnUpdate` ops to it. The frontend ONLY renders — no fence parsing, label
// formatting or phase machine here (docs/agents/ask-ai-streaming.md).
import type {
  AnswerBlock,
  AskAiSource,
  ConversationTurn,
  ToolActivityEntry,
  TurnErrorKind,
  TurnUpdate,
  TurnView,
} from "$lib/insights/conversation";

export interface ChatTurn {
  turnIndex: number;
  question: string;
  phase: "thinking" | "streaming" | "done" | "error";
  /** Render-ready answer blocks (prose stays raw markdown for AnswerProse). */
  blocks: AnswerBlock[];
  reasoning: string | null;
  toolActivities: ToolActivityEntry[];
  /** The in-flight tool call's working line (cleared on done). */
  liveActivity: ToolActivityEntry | null;
  sources: AskAiSource[];
  errorMessage: string | null;
  /** `TurnUpdate::Error.kind` from the live stream (not persisted). */
  errorKind: TurnErrorKind | null;
  /** An error caught by send() before the backend saved a row (CH-04): it must
   *  not count toward turnIndex. */
  localOnly?: boolean;
  /** Provider-reported context-window tokens after the latest completion; null
   *  on cold-loaded turns (usage isn't persisted). */
  contextTokens: number | null;
  /** Last applied `ask_ai_update` version (0 for a hydrated, non-live turn). */
  version: number;
  /** UI-only: when this session sent the turn, and how long it took to finish
   *  (the steps line's duration; never persisted). */
  startedAtMs: number | null;
  elapsedMs: number | null;
  /** When the question was asked (persisted createdAtMs, or now for a live one). */
  atMs: number | null;
}

export function makeTurn(turnIndex: number, question: string, phase: ChatTurn["phase"]): ChatTurn {
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
    errorKind: null,
    contextTokens: null,
    version: 0,
    startedAtMs: null,
    elapsedMs: null,
    atMs: null,
  };
}

export function normalizePhase(phase: string): ChatTurn["phase"] {
  return phase === "done" || phase === "error" || phase === "streaming" || phase === "thinking"
    ? phase
    : "done";
}

/** Persisted tool activity is still the raw `{tool, params}` shape; map it to
 *  minimal render-ready entries so a reloaded turn's steps still count. Live and
 *  snapshot views deliver fully render-ready entries instead. */
export function coerceToolActivities(value: unknown): ToolActivityEntry[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((e): ToolActivityEntry | null => {
      if (typeof e !== "object" || e === null) return null;
      const rec = e as { tool?: unknown; kind?: unknown; label?: unknown };
      if (typeof rec.label === "string" && typeof rec.kind === "string") {
        return { kind: rec.kind, label: rec.label };
      }
      const tool = typeof rec.tool === "string" ? rec.tool : null;
      if (tool === "search") return { kind: "search", label: "Searched your captures" };
      if (tool === "timeline") return { kind: "timeline", label: "Scanned timeline" };
      if (tool === "show_text") return { kind: "show_text", label: "Read a capture" };
      if (tool === "recall_context") return { kind: "recall_context", label: "Recalled what I know about you" };
      return { kind: "other", label: tool ? `Ran ${tool}` : "Working" };
    })
    .filter((x): x is ToolActivityEntry => x !== null);
}

export function coerceSources(value: unknown): AskAiSource[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (s): s is AskAiSource =>
      typeof s === "object" &&
      s !== null &&
      ((s as AskAiSource).kind === "frame" || (s as AskAiSource).kind === "audio"),
  );
}

/** A persisted turn → ChatTurn. `get_conversation` fills `blocks` for every turn
 *  (legacy ones parsed on read), so nothing is parsed here. */
export function hydrateTurn(turn: ConversationTurn): ChatTurn {
  const t = makeTurn(turn.turnIndex, turn.question, normalizePhase(turn.phase));
  t.blocks = turn.blocks ?? [];
  t.reasoning = turn.reasoning;
  t.toolActivities = coerceToolActivities(turn.toolActivities);
  t.sources = coerceSources(turn.sources);
  t.errorMessage = turn.errorMessage;
  t.atMs = turn.createdAtMs;
  return t;
}

/** Replace a turn's render fields from a backend `TurnView` (snapshot adopt). */
export function adoptView(turn: ChatTurn, view: TurnView, version: number): void {
  turn.phase = normalizePhase(view.phase);
  turn.blocks = view.blocks;
  turn.reasoning = view.reasoning;
  turn.toolActivities = view.toolActivities;
  turn.liveActivity = view.liveActivity;
  turn.sources = coerceSources(view.sources);
  turn.errorMessage = view.errorMessage;
  turn.contextTokens = view.contextTokens;
  turn.version = version;
}

/** Apply one streamed op. MUST stay a 1:1 mirror of the Rust
 *  `apply_update_to_view` (esp. AppendProse coalescing) or live and reload
 *  diverge. Assigns fresh arrays so a `$state` turn notices the change. */
export function applyUpdate(turn: ChatTurn, update: TurnUpdate): void {
  switch (update.op) {
    case "phase":
      turn.phase = normalizePhase(update.phase);
      break;
    case "appendProse": {
      const last = turn.blocks[turn.blocks.length - 1];
      turn.blocks =
        last && last.kind === "prose"
          ? [...turn.blocks.slice(0, -1), { kind: "prose", markdown: last.markdown + update.text }]
          : [...turn.blocks, { kind: "prose", markdown: update.text }];
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
      break;
    case "contextTokens":
      turn.contextTokens = update.tokens;
      break;
    case "error":
      turn.errorMessage = update.message;
      turn.errorKind = update.kind ?? null;
      turn.phase = "error";
      break;
    case "done":
      turn.phase = "done";
      turn.liveActivity = null;
      if (turn.startedAtMs !== null) turn.elapsedMs = Date.now() - turn.startedAtMs;
      break;
  }
}

/** The answer's copyable text: the prose blocks' raw markdown. */
export function answerPlainText(turn: ChatTurn): string {
  return turn.blocks
    .filter((b): b is { kind: "prose"; markdown: string } => b.kind === "prose")
    .map((b) => b.markdown)
    .join("\n\n")
    .trim();
}
