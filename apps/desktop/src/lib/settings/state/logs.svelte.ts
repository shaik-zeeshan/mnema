// Developer-panel log state: the native-capture debug log and the general app
// log. Owns its own non-draft reactive state and the load/open/delete invokes.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { GeneralAppLogStatus, NativeCaptureDebugLogStatus } from "$lib/types";
import { errorText } from "./format";

export function createLogsStore() {
  // Debug log status
  let debugLogStatus = $state<NativeCaptureDebugLogStatus | null>(null);
  let loadingDebugLogStatus = $state(false);
  let openingDebugLog = $state(false);
  let deletingDebugLog = $state(false);
  let debugLogError = $state<string | null>(null);
  let debugLogDeleted = $state(false);

  // General app log status
  let generalLogStatus = $state<GeneralAppLogStatus | null>(null);
  let loadingGeneralLogStatus = $state(false);
  let openingGeneralLog = $state(false);
  let deletingGeneralLog = $state(false);
  let generalLogError = $state<string | null>(null);
  let generalLogDeleted = $state(false);

  async function loadDebugLogStatus() {
    loadingDebugLogStatus = true;
    debugLogError = null;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>(
        "get_native_capture_debug_log_status",
      );
    } catch (err) {
      debugLogError = errorText(err);
    } finally {
      loadingDebugLogStatus = false;
    }
  }

  async function openDebugLog() {
    openingDebugLog = true;
    debugLogError = null;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>(
        "open_native_capture_debug_log",
      );
    } catch (err) {
      debugLogError = errorText(err);
    } finally {
      openingDebugLog = false;
    }
  }

  async function deleteDebugLog() {
    const ok = await ask("Delete the native capture debug log file?", {
      title: "Delete native capture debug log",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingDebugLog = true;
    debugLogError = null;
    debugLogDeleted = false;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>(
        "delete_native_capture_debug_log",
      );
      debugLogDeleted = true;
      setTimeout(() => { debugLogDeleted = false; }, 2200);
    } catch (err) {
      debugLogError = errorText(err);
    } finally {
      deletingDebugLog = false;
    }
  }

  async function loadGeneralLogStatus() {
    loadingGeneralLogStatus = true;
    generalLogError = null;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("get_general_app_log_status");
    } catch (err) {
      generalLogError = errorText(err);
    } finally {
      loadingGeneralLogStatus = false;
    }
  }

  async function openGeneralLog() {
    openingGeneralLog = true;
    generalLogError = null;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("open_general_app_log");
    } catch (err) {
      generalLogError = errorText(err);
    } finally {
      openingGeneralLog = false;
    }
  }

  async function deleteGeneralLog() {
    const ok = await ask("Delete the general application log file?", {
      title: "Delete general application log",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingGeneralLog = true;
    generalLogError = null;
    generalLogDeleted = false;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("delete_general_app_log");
      generalLogDeleted = true;
      setTimeout(() => { generalLogDeleted = false; }, 2200);
    } catch (err) {
      generalLogError = errorText(err);
    } finally {
      deletingGeneralLog = false;
    }
  }

  return {
    get debugLogStatus() { return debugLogStatus; },
    get loadingDebugLogStatus() { return loadingDebugLogStatus; },
    get openingDebugLog() { return openingDebugLog; },
    get deletingDebugLog() { return deletingDebugLog; },
    get debugLogError() { return debugLogError; },
    set debugLogError(v: string | null) { debugLogError = v; },
    get debugLogDeleted() { return debugLogDeleted; },
    get generalLogStatus() { return generalLogStatus; },
    get loadingGeneralLogStatus() { return loadingGeneralLogStatus; },
    get openingGeneralLog() { return openingGeneralLog; },
    get deletingGeneralLog() { return deletingGeneralLog; },
    get generalLogError() { return generalLogError; },
    set generalLogError(v: string | null) { generalLogError = v; },
    get generalLogDeleted() { return generalLogDeleted; },
    loadDebugLogStatus,
    openDebugLog,
    deleteDebugLog,
    loadGeneralLogStatus,
    openGeneralLog,
    deleteGeneralLog,
  };
}

export type LogsStore = ReturnType<typeof createLogsStore>;
