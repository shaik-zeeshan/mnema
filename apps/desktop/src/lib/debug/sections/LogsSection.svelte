<script lang="ts">
  // Logs — the mockup's two config rows (general app log: tail/open/delete;
  // native capture debug log: active badge + enable/disable) over the shared
  // <LogTail>, which "Tail in app" reveals on demand.
  //
  // Open/delete/status reuse the settings Developer panel's store wholesale —
  // same commands, same confirm dialogs — rather than re-invoking by hand.

  import { invoke } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import { createLogsStore } from "$lib/settings/state/logs.svelte";
  import { errorText } from "$lib/settings/state/format";
  import LogTail from "../LogTail.svelte";
  import { formatBytes, shortenPath } from "../format";
  import { anchor } from "../sections";

  const logs = createLogsStore();

  let showTail = $state(false);
  let togglingCaptureLog = $state(false);

  $effect(() => {
    void logs.loadGeneralLogStatus();
    void logs.loadDebugLogStatus();
  });

  const general = $derived(logs.generalLogStatus);
  const capture = $derived(logs.debugLogStatus);

  /** Mockup desc: `~/Library/Logs/com.mnema.app/rust.log · 12 MB`. */
  const generalDesc = $derived(
    general?.sizeBytes != null
      ? `${shortenPath(general.path)} · ${formatBytes(general.sizeBytes)}`
      : shortenPath(general?.path),
  );

  /** Flip the developer-options setting the log is gated on; the backend
   *  reconfigures the writer immediately (no restart), then re-read status. */
  async function toggleCaptureLog() {
    if (!capture || togglingCaptureLog) return;
    togglingCaptureLog = true;
    logs.debugLogError = null;
    try {
      await invoke("update_developer_settings", {
        request: { nativeCaptureDebugLoggingEnabled: !capture.enabled },
      });
      await logs.loadDebugLogStatus();
    } catch (err) {
      logs.debugLogError = errorText(err);
    } finally {
      togglingCaptureLog = false;
    }
  }
</script>

<SettingGroup
  title="Logs"
  hint="rust.log · native-capture-debug.log"
  hintInline
  id={anchor("logs")}
>
  <div class="row">
    <div class="row__main">
      <div class="row__label">General application log</div>
      <div class="row__desc row__desc--mono" use:tip={general?.path ?? ""}>{generalDesc}</div>
    </div>
    <div class="row__value">
      <button
        type="button"
        class="btn"
        aria-pressed={showTail}
        onclick={() => (showTail = !showTail)}
      >
        Tail in app
      </button>
      <button
        type="button"
        class="btn"
        disabled={logs.openingGeneralLog}
        onclick={logs.openGeneralLog}
      >
        Open
      </button>
      <button
        type="button"
        class="btn btn--danger"
        disabled={logs.deletingGeneralLog || general?.exists === false}
        onclick={logs.deleteGeneralLog}
      >
        {logs.generalLogDeleted ? "Deleted" : "Delete"}
      </button>
    </div>
  </div>
  {#if logs.generalLogError}
    <p class="debug-errline" role="alert" aria-live="polite">{logs.generalLogError}</p>
  {/if}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Native capture debug log</div>
      <div class="row__desc" use:tip={capture?.path ?? ""}>
        verbose capture diagnostics · currently {capture?.enabled ? "on" : "off"}
      </div>
    </div>
    <div class="row__value">
      <span class={capture?.enabled ? "badge badge--ok" : "badge badge--neutral"}>
        {capture?.enabled ? "Active" : "Inactive"}
      </span>
      <button
        type="button"
        class="btn"
        disabled={capture == null || togglingCaptureLog}
        onclick={toggleCaptureLog}
      >
        {capture?.enabled ? "Disable" : "Enable"}
      </button>
    </div>
  </div>
  {#if logs.debugLogError}
    <p class="debug-errline" role="alert" aria-live="polite">{logs.debugLogError}</p>
  {/if}

  {#if showTail}
    <div class="debug-body">
      <LogTail />
    </div>
  {/if}
</SettingGroup>
