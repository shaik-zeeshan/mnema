// Pure list helpers for the model picker's starred and recent models. Both
// lists persist in the AI runtime settings (`favoriteModels`, `recentModels`);
// the backend also trims, de-duplicates, drops disconnected providers and caps
// recents, so these only need to produce the intended next list.
import type { AccessSettings, AiEngineRef, AiRuntimeSettings } from "$lib/types/recording";

/** How many recent models the backend keeps (mirrors AI_RECENT_MODELS_CAP). */
export const RECENT_MODELS_CAP = 5;
/** How many recent models the picker's short list shows. */
export const QUICK_RECENTS = 2;

/** A plain `{provider, model}` copy (the inputs may be reactive proxies of a settings snapshot). */
const plain = (e: AiEngineRef): AiEngineRef => ({ provider: e.provider, model: e.model });

export function sameEngine(a: AiEngineRef | null | undefined, b: AiEngineRef | null | undefined): boolean {
  return !!a && !!b && a.provider === b.provider && a.model === b.model;
}

export function isFavorite(favorites: AiEngineRef[], engine: AiEngineRef): boolean {
  return favorites.some((f) => sameEngine(f, engine));
}

/** Star (append, keeping star order) or unstar `engine`. */
export function toggleFavorite(favorites: AiEngineRef[], engine: AiEngineRef): AiEngineRef[] {
  return isFavorite(favorites, engine)
    ? favorites.filter((f) => !sameEngine(f, engine)).map(plain)
    : [...favorites, engine].map(plain);
}

/** Move `engine` to the front of the recents, capped. */
export function pushRecent(recents: AiEngineRef[], engine: AiEngineRef): AiEngineRef[] {
  return [engine, ...recents.filter((r) => !sameEngine(r, engine))].slice(0, RECENT_MODELS_CAP).map(plain);
}

/** The recents worth showing next to the favorites: not starred, first `n`. */
export function quickRecents(recents: AiEngineRef[], favorites: AiEngineRef[], n = QUICK_RECENTS): AiEngineRef[] {
  return recents.filter((r) => !isFavorite(favorites, r)).slice(0, n);
}

/** The favorite after `current` (wrapping); the first one when `current` isn't starred. */
export function nextFavorite(favorites: AiEngineRef[], current: AiEngineRef | null): AiEngineRef | null {
  if (favorites.length === 0) return null;
  const at = favorites.findIndex((f) => sameEngine(f, current));
  return favorites[(at + 1) % favorites.length];
}

/**
 * What Ask AI's "Default" runs (no thread pin) — the frontend mirror of the
 * backend `ask_ai_resolver_inputs` + resolver: the Ask AI model override on its
 * saved provider (skipped when that provider is gone), a bare override on the
 * global default's provider, else the global default model.
 */
export function askAiDefaultEngine(
  aiRuntime: AiRuntimeSettings | null | undefined,
  access: Pick<AccessSettings, "askAiModel" | "askAiModelProvider"> | null | undefined,
): AiEngineRef | null {
  const def = aiRuntime?.defaultModel?.model.trim() ? aiRuntime.defaultModel : null;
  const model = access?.askAiModel?.trim() ?? "";
  const provider = access?.askAiModelProvider?.trim() ?? "";
  if (!model) return def;
  if (provider) return aiRuntime?.providers.some((p) => p.id === provider) ? { provider, model } : def;
  return def ? { provider: def.provider, model } : null;
}
