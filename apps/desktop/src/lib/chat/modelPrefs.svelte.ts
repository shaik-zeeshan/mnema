// ModelPrefs — the model picker's starred + recent models and "make default",
// read from whatever AI runtime snapshot the surface already holds (Chat's
// settings load, Settings' canonical store) and saved straight through
// `update_ai_runtime_settings` with ONLY the changed field, so it never races
// the Settings page's provider/default-model autosave.
//
// Writes are optimistic: the new list shows at once and stays until the
// surface's snapshot catches up (the save fires `recording_settings_domain_
// changed`, which every surface already listens to). Construct during
// component init — it registers an effect.
import { invoke } from "@tauri-apps/api/core";
import type {
  AiEngineRef,
  AiRuntimeSettings,
  RecordingSettingsDomainUpdateResponse,
  UpdateAiRuntimeSettingsRequest,
} from "$lib/types/recording";
import { pushRecent, sameEngine, toggleFavorite } from "./model-favorites";

const sameList = (a: AiEngineRef[], b: AiEngineRef[]) => a.length === b.length && a.every((x, i) => sameEngine(x, b[i]));

export class ModelPrefs {
  #source: () => AiRuntimeSettings | null | undefined;
  #favs = $state.raw<AiEngineRef[] | null>(null); // raw: compared by identity below
  #recents = $state.raw<AiEngineRef[] | null>(null);

  favorites = $derived(this.#favs ?? this.#sourceFavs());
  recents = $derived(this.#recents ?? this.#sourceRecents());

  constructor(source: () => AiRuntimeSettings | null | undefined) {
    this.#source = source;
    // Drop an optimistic list once the snapshot holds the same thing (a later
    // in-flight save keeps its own list until its own snapshot lands).
    $effect(() => {
      const favs = this.#sourceFavs();
      const recents = this.#sourceRecents();
      if (this.#favs && sameList(this.#favs, favs)) this.#favs = null;
      if (this.#recents && sameList(this.#recents, recents)) this.#recents = null;
    });
  }

  #sourceFavs(): AiEngineRef[] {
    return this.#source()?.favoriteModels ?? [];
  }
  #sourceRecents(): AiEngineRef[] {
    return this.#source()?.recentModels ?? [];
  }

  async #save(request: UpdateAiRuntimeSettingsRequest): Promise<AiRuntimeSettings | null> {
    try {
      const response = await invoke<RecordingSettingsDomainUpdateResponse>("update_ai_runtime_settings", { request });
      return response.settings.aiRuntime;
    } catch (error) {
      console.warn("model picker: saving AI settings failed", error);
      return null;
    }
  }

  toggleFavorite(engine: AiEngineRef): void {
    const next = toggleFavorite(this.favorites, engine);
    this.#favs = next;
    // Settle on what was actually saved (the backend drops refs to providers
    // that aren't saved yet); on failure fall back to the snapshot.
    void this.#save({ favoriteModels: next }).then((saved) => {
      if (this.#favs === next) this.#favs = saved ? saved.favoriteModels : null;
    });
  }

  /** Record a model picked in Chat (no-op when it's already the most recent). */
  pushRecent(engine: AiEngineRef): void {
    if (sameEngine(this.recents[0], engine)) return;
    const next = pushRecent(this.recents, engine);
    this.#recents = next;
    void this.#save({ recentModels: next }).then((saved) => {
      if (this.#recents === next) this.#recents = saved ? saved.recentModels : null;
    });
  }

  /** Make `engine` the global default model (what new chats use). */
  async makeDefault(engine: AiEngineRef): Promise<void> {
    await this.#save({ defaultModel: { provider: engine.provider, model: engine.model } });
  }
}
