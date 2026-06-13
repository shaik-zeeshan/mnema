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
  /** The model's reasoning ("thinking") text for this turn, or null when none. */
  reasoning: string | null;
  /** Render-ready parsed answer blocks (the backend-owned render model); null
   *  for a legacy turn the backend parses from `answer` on read. The
   *  get_conversation command always populates this, so the UI never parses. */
  blocks: AnswerBlock[] | null;
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
  /** Pinned engine provider id, or null/absent when unpinned (global default). */
  provider?: string | null;
  /** Pinned model id within `provider`, or null/absent when unpinned. */
  model?: string | null;
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

// ── Render-ready chat view model (issue #110, Slice 1) ───────────────────────
// The BACKEND-OWNED render model for a streaming Ask AI turn, mirroring the Rust
// types in `crates/capture-types/src/conversation.rs`. The backend decides what
// a turn looks like (phases, answer blocks, reasoning, tool activity); the
// frontend ONLY renders. There is no codegen — these must agree field-for-field
// with the Rust side, which a serde round-trip test guards.
//
// Option fields that Rust SKIPS when absent (item/block options) are typed
// optional here (`?`); the TurnView's nullable fields always serialize (as null)
// and are typed `T | null`.

/** One row of a horizontal-bar answer block. */
export interface BarsItem {
  label: string;
  value: number;
  sublabel?: string;
}

/** One claim/finding in a dossier block. `confidence` is a 0..1 score. */
export interface DossierItem {
  subject?: string | null;
  statement: string;
  confidence: number;
}

/** One interval in a timeline block. `end` absent = open/instant interval. */
export interface TimelineItem {
  label: string;
  start: string;
  end?: string | null;
  app?: string | null;
  category?: string | null;
}

/** One render-ready answer block, discriminated on `kind`.
 *
 * `prose` carries RAW markdown — the markdown→HTML pass stays on the frontend
 * (AnswerProse). The graphical variants carry already-parsed data, so the UI no
 * longer re-parses fenced JSON. */
export type AnswerBlock =
  | { kind: "prose"; markdown: string }
  | { kind: "bars"; title?: string | null; items: BarsItem[] }
  | { kind: "dossier"; items: DossierItem[] }
  | { kind: "timeline"; title?: string | null; items: TimelineItem[] };

/** One recorded brokered tool call rendered in the turn's activity rail.
 *  `kind` mirrors the Rust `String` (a frontend `AskToolKind` union member). */
export interface ToolActivityEntry {
  kind: string;
  label: string;
  /** The app the call was scoped to (bundle id or display name), if any. */
  app?: string | null;
  /** A resolved icon path for `app`, when the backend found one. */
  appIconPath?: string | null;
}

/** The full render-ready view of ONE Ask AI turn. The backend owns every field;
 *  the frontend only renders. `phase` is the lifecycle string. `sources` is the
 *  opaque Answer-Sources JSON the UI round-trips. The nullable fields always
 *  serialize (as null), hence `T | null` rather than optional. */
export interface TurnView {
  turnIndex: number;
  question: string;
  /** `"seeding" | "thinking" | "streaming" | "done" | "error"`. */
  phase: string;
  blocks: AnswerBlock[];
  reasoning: string | null;
  toolActivities: ToolActivityEntry[];
  liveActivity: ToolActivityEntry | null;
  sources: unknown;
  errorMessage: string | null;
  seededResultCount: number | null;
}

/** A versioned snapshot of a turn's `TurnView`. `version` lets the frontend
 *  discard stale snapshots that race with live `TurnUpdate`s. */
export interface TurnSnapshot {
  conversationId: string;
  version: number;
  view: TurnView;
}

/** One incremental mutation to a `TurnView`, discriminated on `op`. The backend
 *  streams these as a turn progresses; the frontend applies them to its local
 *  view. (`liveActivity` with `entry: null` CLEARS the live line.) */
export type TurnUpdate =
  | { op: "phase"; phase: string }
  | { op: "appendProse"; text: string }
  | { op: "openBlock"; block: AnswerBlock }
  | { op: "reasoning"; text: string }
  | { op: "toolActivity"; entry: ToolActivityEntry }
  | { op: "liveActivity"; entry: ToolActivityEntry | null }
  | { op: "sources"; sources: unknown }
  | { op: "error"; message: string }
  | { op: "done" };

/** The `ask_ai_update` event payload (emitted by Slice 4): a versioned, indexed
 *  `TurnUpdate` for a conversation. Emitted ad-hoc as JSON on the Rust side, so
 *  it has no capture-types struct — this TS type is the sole wire contract. */
export interface AskAiUpdateEvent {
  conversationId: string;
  version: number;
  turnIndex: number;
  update: TurnUpdate;
}

// ── Ask AI availability + Answer Sources ─────────────────────────────────────
// The legacy per-token `ask_ai_*` event shapes (status/delta/reasoning/done/
// error/source) are gone: BOTH doors (Chat and Quick Recall) now consume the
// single versioned `ask_ai_update` transport (AskAiUpdateEvent above), and the
// backend no longer emits the legacy events. Availability + the cited Answer
// Source shape remain, shared across doors.

export interface AskAiAvailability {
  available: boolean;
  reason: string | null;
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

// ── Per-thread engine pin (set_conversation_engine) ──────────────────────────
// A conversation can be pinned to one engine identity `{provider, model}` from
// the provider-tagged model pool (ADR 0034). The pin is identified by a connected
// provider **instance id** (`AiProviderConfig.id` — equal to the kind id for the
// first instance of a kind, suffixed for additional same-kind instances) plus the
// bare rig-core model id, matching the Rust single resolver
// (`resolve_engine_config`): thread pin → feature override → global default.

import type {
  AiProviderConfig,
  AiRuntimeModel,
  AiRuntimeSettings,
} from "$lib/types/recording";

/** One selectable engine for the per-thread picker. */
export interface PinnableEngine {
  /** Stable engine-pin provider string passed to `set_conversation_engine`. */
  provider: string;
  /** Bare rig-core model id passed to `set_conversation_engine`. */
  model: string;
  /** Human label, e.g. "Anthropic · claude-haiku-4-5". */
  label: string;
}

/** A short, legible label for a model id: the tail after the last "/",
 *  so long routing paths (accounts/.../deepseek-v4-pro) read cleanly. The
 *  full id stays available for a title= tooltip. */
export function shortModelLabel(model: string): string {
  const trimmed = model.trim();
  const tail = trimmed.split("/").pop()?.trim() ?? "";
  return tail.length > 0 ? tail : trimmed;
}

/** A short, friendly label for one provider KIND id. Returns the raw string for
 *  an unknown id (e.g. a `kind-N` instance id with no provider context). */
export function engineProviderLabel(provider: string): string {
  switch (provider) {
    case "anthropic":
      return "Anthropic";
    case "openai":
      return "OpenAI";
    case "openai_compatible":
      return "OpenAI-compatible";
    case "ollama":
      return "Ollama";
    case "llamafile":
      return "Llamafile";
    default:
      return provider;
  }
}

/** Host component of a base URL, for the auto-derived instance label. */
function baseUrlHost(baseUrl: string): string {
  const trimmed = baseUrl.trim();
  if (!trimmed) return "";
  try {
    return new URL(trimmed).host || trimmed;
  } catch {
    return trimmed;
  }
}

/** Display name for a provider instance (mirrors the Settings card): its user
 *  label, else kind + host, else kind + instance number. Keeps two same-kind
 *  instances distinct even with no label typed, including first-party cloud
 *  (anthropic/openai) which has no base-URL host to disambiguate on. */
export function providerInstanceLabel(provider: AiProviderConfig): string {
  const label = provider.label?.trim() ?? "";
  if (label) return label;
  const kindLabel = engineProviderLabel(provider.kind);
  const host = baseUrlHost(provider.baseUrl ?? "");
  if (host) return `${kindLabel} · ${host}`;
  const suffix = provider.id.startsWith(`${provider.kind}-`)
    ? provider.id.slice(provider.kind.length + 1)
    : "";
  return suffix ? `${kindLabel} (${suffix})` : kindLabel;
}

/** Resolve a provider INSTANCE id to its display label against the connected
 *  provider list. Falls back to the kind label for an unknown/removed instance
 *  (e.g. a pin to a provider that has since been disconnected). */
export function providerLabelById(
  providers: readonly AiProviderConfig[] | null | undefined,
  id: string,
): string {
  const provider = providers?.find((p) => p.id === id);
  return provider ? providerInstanceLabel(provider) : engineProviderLabel(id);
}

/** The global default model's provider id, or null when no default is chosen. */
export function defaultEnginePinProvider(
  settings: AiRuntimeSettings,
): string | null {
  return settings.defaultModel?.provider ?? null;
}

/**
 * The model id the global default resolves to, for LABEL purposes only
 * (e.g. the composer picker's "Default · claude-haiku-4-5"); the backend owns
 * actual resolution. Null when no default model is chosen.
 */
export function defaultEngineModel(settings: AiRuntimeSettings): string | null {
  const model = settings.defaultModel?.model?.trim() ?? "";
  return model.length > 0 ? model : null;
}

/**
 * Map the merged provider-tagged model pool (`ai_runtime_list_models`) into
 * pinnable engines for the per-thread picker, de-duplicated by
 * `(provider, model)`. Free-form model-id entry stays allowed on top of this.
 */
export function pinnableEnginesFromModelPool(
  models: readonly AiRuntimeModel[],
  providers?: readonly AiProviderConfig[] | null,
): PinnableEngine[] {
  const seen = new Set<string>();
  const out: PinnableEngine[] = [];
  for (const entry of models) {
    const model = entry.id.trim();
    if (model.length === 0) continue;
    const key = `${entry.provider} ${model}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      provider: entry.provider,
      model,
      label: `${providerLabelById(providers, entry.provider)} · ${model}`,
    });
  }
  return out;
}
