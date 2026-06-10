// Persistent conversation DTOs (issue #102/#110, ADR 0031) — the frontend
// mirror of `crates/capture-types/src/conversation.rs`. ONE shared conversation
// store backs both doors (Quick Recall and Chat); these types are the wire shape
// of its Tauri commands (`list_conversations` / `get_conversation` /
// `search_conversations` / `save_conversation_turn` / `delete_conversation`).
//
// `toolActivities` / `sources` are opaque JSON the frontend round-trips, so they
// are typed `unknown` here and re-narrowed where consumed.

/** A lightweight conversation row for the history list. */
export interface ConversationSummary {
  conversationId: string;
  title: string;
  /** The door that created it: `"quick_recall"` | `"chat"`. */
  origin: string;
  createdAtMs: number;
  updatedAtMs: number;
  turnCount: number;
  /** The first turn's question, truncated. */
  preview: string;
}

/** One persisted question/answer turn, in `turnIndex` order. */
export interface ConversationTurn {
  turnIndex: number;
  question: string;
  answer: string;
  /** Opaque per-turn tool-activity log (JSON array round-tripped by the UI). */
  toolActivities: unknown;
  /** Opaque per-turn Answer Sources (JSON array round-tripped by the UI). */
  sources: unknown;
  /** `"streaming"` | `"done"` | `"error"`. */
  phase: string;
  errorMessage: string | null;
  seededResultCount: number | null;
  createdAtMs: number;
  updatedAtMs: number;
}

/** A fully-hydrated persisted conversation: metadata + every turn. */
export interface Conversation {
  conversationId: string;
  title: string;
  origin: string;
  createdAtMs: number;
  updatedAtMs: number;
  turns: ConversationTurn[];
}

/** The conversation door that owns a saved turn: the Insights Chat workspace or
 *  the Quick Recall launcher (both now persist to the shared store — #111). */
export type ConversationOrigin = "chat" | "quick_recall";

/** The payload sent to `save_conversation_turn` (upserts the row + the turn). */
export interface SaveConversationTurnRequest {
  conversationId: string;
  title: string;
  origin: ConversationOrigin;
  turnIndex: number;
  question: string;
  answer: string;
  toolActivities: unknown;
  sources: unknown;
  phase: string;
  errorMessage: string | null;
  seededResultCount: number | null;
}

// ── Ask AI streaming event shapes (shared with Quick Recall) ─────────────────
// These mirror the snake_case-camelCase payloads the Rust host emits over the
// `ask_ai_*` events. Kept here so Chat and any other Ask AI door agree on shape.

export interface AskAiAvailability {
  available: boolean;
  reason: string | null;
}

export interface AskAiStatusEvent {
  conversationId: string;
  phase: "seeding" | "thinking" | "tool";
  seededResultCount?: number;
  tool?: string;
  params?: Record<string, unknown>;
}

export interface AskAiDeltaEvent {
  conversationId: string;
  text: string;
}

export interface AskAiDoneEvent {
  conversationId: string;
}

export interface AskAiErrorEvent {
  conversationId: string;
  message: string;
}

/** One cited Answer Source (a captured frame or audio segment). */
export interface AskAiSource {
  kind: "frame" | "audio";
  frameId: number | null;
  audioSegmentId: number | null;
  appName: string | null;
  windowTitle: string | null;
  startedAt: string;
  endedAt: string;
  sourceKind: "microphone" | "system" | null;
  spanStartMs?: number | null;
  alignedFrameId?: number | null;
}

export interface AskAiSourceEvent {
  conversationId: string;
  sources: AskAiSource[];
}

/** One recorded brokered tool call: kind + a humane filter label. */
export type AskToolKind = "search" | "timeline" | "show_text" | "other";
export interface AskToolActivityEntry {
  kind: AskToolKind;
  label: string;
}

