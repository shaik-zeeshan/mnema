// Stubs Tauri IPC in a plain browser so Playwright can screenshot the Svelte app.
// Recipe: docs/mockups/redesign/DEVELOPER-PLAN.md:95-117.
import type { Page } from "@playwright/test";

/** command -> the JSON its `invoke` resolves with. */
export type CommandMap = Record<string, unknown>;

export interface StubOptions {
  /** Tauri window label (`main`, `debug`, `quick-recall`, ...). */
  label: string;
  map: CommandMap;
}

/** Browser-side stub, serialised into the page by `addInitScript`. Must stay self-contained. */
function stub({ label, map }: StubOptions) {
  // Unmocked commands get an empty value of their real return type. Never `null` where the Rust type
  // isn't an Option: a null where the page wants an array or object throws inside an `$effect` and
  // silently freezes the whole page. And never `{}` for an Option: it reads as "Some" (a pending
  // license deep link opens the license-receipt overlay over every page; a pending CLI request renders
  // garbage). Tables are the non-object returns of the 248 #[tauri::command]s as of 2026-11-08;
  // anything else falls back to the name rules, then `{}`.
  // shortcut: hand-copied from the Rust signatures; S1's fixture dumper makes these maps generated.
  const nullable = new Set([
    "ai_runtime_provider_key_ending", "ask_ai_snapshot", "get_account_owner_person_id", "get_app_job",
    "get_ask_ai_last_turn_usage", "get_audio_segment", "get_conversation", "get_earliest_earlier_equivalent_frame",
    "get_frame", "get_frame_preview", "get_latest_frame_in_range", "get_license_devices",
    "get_microphone_activity_level", "get_nearest_earlier_equivalent_frame", "get_pending_cli_access_request",
    "get_processing_job", "get_processing_result", "get_system_audio_probe_level", "mcp_check_node",
    "system_uses_24_hour_time", "take_pending_license_deep_link", "take_quick_recall_query",
    "transcription_deepgram_auth_status",
  ]);
  // `Vec` returns without a `list_` name. `drain_*` are Vecs: the page does `for (const p of payloads)`.
  const arrays = new Set([
    "clear_app_notification", "clear_app_notifications", "drain_pending_broker_open_capture_results",
    "drain_pending_insights_open_conversations", "drain_pending_open_settings", "get_app_notifications",
    "get_audio_segment_waveform_peaks", "get_capture_hours", "get_debug_health", "get_frame_match_boxes",
    "get_processing_pipeline_status", "get_quick_recall_recent_queries", "mcp_list_server_tools",
    "mcp_oauth_statuses", "resolve_app_icons", "search_conversations", "set_quick_recall_recent_queries",
    "user_context_list_dismissed",
  ]);
  const scalars: Record<string, unknown> = {
    ai_runtime_has_provider_key: false, block_cli_access_client: false, mcp_has_server_secret: false,
    open_captured_url: false, transcription_has_deepgram_key: false, unblock_cli_access_client: false,
    cancel_active_frame_preview_video_requests: 0, select_semantic_search_model: 0,
    get_storage_location: "", record_bounded_microphone_clip: "",
  };
  const fallback = (cmd: string): unknown => {
    const name = cmd.includes("|") ? cmd.split("|")[1] : cmd;
    if (nullable.has(name)) return null;
    if (name in scalars) return scalars[name];
    if (name.startsWith("list_") || name.startsWith("drain_") || arrays.has(name)) return [];
    if (name.startsWith("has_pending")) return false;
    return {};
  };
  const calls: { cmd: string; mocked: boolean }[] = [];
  let nextCallback = 0;
  const w = window as unknown as Record<string, unknown>;
  w.__parityCalls = calls;
  w.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string) => {
      const mocked = Object.prototype.hasOwnProperty.call(map, cmd);
      calls.push({ cmd, mocked });
      const value = mocked ? map[cmd] : fallback(cmd);
      // `{ "__reject": "<reason>" }` makes the command fail with that reason.
      if (value && typeof value === "object" && "__reject" in value) throw (value as { __reject: string }).__reject;
      return structuredClone(value);
    },
    transformCallback: (callback: (v: unknown) => void, once = false) => {
      const id = ++nextCallback;
      w[`_${id}`] = (v: unknown) => {
        if (once) delete w[`_${id}`];
        callback?.(v);
      };
      return id;
    },
    unregisterCallback: (id: number) => delete w[`_${id}`],
    // Tauri's asset:// doesn't exist in a browser; installTauriStub serves these from disk.
    convertFileSrc: (path: string) => `${location.origin}/__parity_file__${path}`,
    metadata: { currentWindow: { label }, currentWebview: { windowLabel: label, label } },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

export async function installTauriStub(page: Page, opts: StubOptions): Promise<void> {
  await page.addInitScript(stub, opts);
  await page.route("**/__parity_file__/**", (route) =>
    route.fulfill({ path: decodeURIComponent(new URL(route.request().url()).pathname.slice("/__parity_file__".length)) }),
  );
}

/** Every command the page invoked, in order, with whether the fixture mocked it. */
export async function invokedCommands(page: Page): Promise<{ cmd: string; mocked: boolean }[]> {
  return page.evaluate(() => (window as unknown as { __parityCalls: { cmd: string; mocked: boolean }[] }).__parityCalls ?? []);
}
