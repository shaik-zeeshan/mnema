// Quick Recall's ask decisions (IMPLEMENTATION.md §Quick Recall "REPLACED
// Ask"), pure so they're tested beside this file: is the typed text a
// question, does Tab force ask or search, which model would answer and does
// "Answer questions automatically" let it answer on its own, and what the hero
// slot says when Ask AI can't answer.
import type { QuickRecallAutoAnswer, RecordingSettings } from "$lib/types/recording";
import { shortModelLabel } from "$lib/insights/conversation";
import { reasonCopy } from "$lib/insights/engine-state";
import { aiProviderInstanceLabel, aiProviderKindLabel } from "$lib/settings/state/ai-providers";
import type { Pill } from "./natural-tokens";
import { pillEditText } from "./filter-chips";

const LEAD =
  /^(who|what|when|where|why|how|did|do|does|was|were|is|are|can|could|should|which|summari[sz]e|remind me|tell me)\b/i;

/** A trailing `?`, or a leading who/what/when… (a bare wh-word alone doesn't
 *  count — it's a word being typed, not a question yet). */
export function isQuestion(text: string): boolean {
  const t = text.trim();
  if (/\?$/.test(t)) return true;
  if (/^\S+$/.test(t) && !/^summari[sz]e$/i.test(t)) return false;
  return LEAD.test(t);
}

/** The question sent to Ask AI: the free text with each pill's phrase put
 *  back ("what did I read in Arc yesterday?"). The same before and after the
 *  phrases turn into pills, so committing them never re-asks. */
export function scopedQuestion(residual: string, pills: readonly Pill[]): string {
  const text = residual.trim();
  const body = [text.replace(/\?+$/, "").trim(), ...pills.map(pillEditText)].filter(Boolean).join(" ");
  return body && text.endsWith("?") ? `${body}?` : body;
}

export type Forced = "ask" | "search" | null;

/** Tab flips what the bar is doing; landing back on what detection would do
 *  on its own clears the force (recall.html `toggleMode`). */
export function nextForced(forced: Forced, autoAsk: boolean): Forced {
  const asking = forced ? forced === "ask" : autoAsk;
  const next = asking ? "search" : "ask";
  return (next === "ask") === autoAsk ? null : next;
}

export interface AskEngine {
  /** Short label of the model Quick Recall would ask, or null with no default. */
  model: string | null;
  /** The resolved provider kind runs on this Mac (Rust `is_local()`). */
  local: boolean;
}

/** The model an Ask AI turn resolves to with no thread pin: the default
 *  model's provider, with the Ask AI override replacing only the model id
 *  (`ai_runtime.rs` `resolve_engine_ref`). */
export function askEngine(settings: RecordingSettings | null): AskEngine {
  const def = settings?.aiRuntime?.defaultModel ?? null;
  const model = settings?.access?.askAiModel?.trim() || def?.model?.trim() || "";
  const kind = settings?.aiRuntime?.providers.find((p) => p.id === def?.provider)?.kind;
  return {
    model: model ? shortModelLabel(model) : null,
    local: kind === "ollama" || kind === "llamafile",
  };
}

export function autoAnswerAllowed(mode: QuickRecallAutoAnswer, engine: AskEngine): boolean {
  return mode === "always" || (mode === "localOnly" && engine.local);
}

export interface AskNotice {
  tone: "" | "off" | "warn" | "danger";
  title: string;
  text: string;
  action: { label: string; primary: boolean } | null;
}

/** Why Ask AI can't answer right now — never a raw reason code. Null = it can. */
export function askNotice(
  reason: string | null,
  settings: RecordingSettings | null,
  online: boolean,
  engine: AskEngine,
): AskNotice | null {
  const ai = settings?.aiRuntime ?? null;
  const name = (id: string) => {
    const p = ai?.providers.find((x) => x.id === id);
    return p ? aiProviderInstanceLabel(p) : aiProviderKindLabel(id);
  };
  const settingsAction = { label: "Turn on in Settings →", primary: false };
  if (reason === null) {
    return online || engine.local
      ? null
      : {
          tone: "warn",
          title: "You’re offline",
          text: `${engine.model ?? "Your model"} runs in the cloud. Search still works — answers resume when you’re back.`,
          action: null,
        };
  }
  if (reason === "ask_ai_disabled" || reason === "ai_runtime_disabled") {
    return { tone: "off", title: "Ask AI is off", text: "Search still works — these are the moments that match.", action: settingsAction };
  }
  if (reason === "no_providers") {
    return {
      tone: "",
      title: "Ask AI needs a provider",
      text: "Add a provider API key in Settings, or run a model on this Mac. The moments below don’t need one.",
      action: { label: "Set up AI", primary: true },
    };
  }
  if (reason.startsWith("needs_reconnect:")) {
    const who = name(reason.slice("needs_reconnect:".length));
    return {
      tone: "danger",
      title: `${who} needs you to sign in again`,
      text: "Your sign-in ended. Reconnect, then ask again.",
      action: { label: "Reconnect", primary: true },
    };
  }
  if (reason.startsWith("provider_unreachable:")) {
    return {
      tone: "warn",
      title: `Can’t reach ${name(reason.slice("provider_unreachable:".length))} right now`,
      text: "Answers resume on their own. The moments below are all on this Mac.",
      action: null,
    };
  }
  const copy = reasonCopy(reason, ai);
  return copy.kind === "unreachable"
    ? { tone: "warn", title: "Can’t reach the model right now", text: copy.text, action: null }
    : { tone: "", title: "Ask AI isn’t set up yet", text: copy.text, action: { label: "Set up AI", primary: true } };
}
