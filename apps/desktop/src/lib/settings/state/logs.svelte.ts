// Developer-panel log state: the general app log. Owns its own non-draft reactive state and the load/open/delete invokes.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type { GeneralAppLogStatus } from "$lib/types";
import { errorText } from "./format";

export function createLogsStore() {
  // General app log status
  let generalLogStatus = $state<GeneralAppLogStatus | null>(null);
  let loadingGeneralLogStatus = $state(false);
  let openingGeneralLog = $state(false);
  let deletingGeneralLog = $state(false);
  let generalLogError = $state<string | null>(null);
  let generalLogDeleted = $state(false);

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
    get generalLogStatus() { return generalLogStatus; },
    get loadingGeneralLogStatus() { return loadingGeneralLogStatus; },
    get openingGeneralLog() { return openingGeneralLog; },
    get deletingGeneralLog() { return deletingGeneralLog; },
    get generalLogError() { return generalLogError; },
    set generalLogError(v: string | null) { generalLogError = v; },
    get generalLogDeleted() { return generalLogDeleted; },
    loadGeneralLogStatus,
    openGeneralLog,
    deleteGeneralLog,
  };
}

export type LogsStore = ReturnType<typeof createLogsStore>;
