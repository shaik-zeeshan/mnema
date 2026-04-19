<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import type {
    ActivityMode,
    CaptureSupport,
    GeneralAppLogStatus,
    NativeCaptureDebugLogStatus,
    RecordingSettings,
    ResolutionMode,
    ResolutionPreset,
    VideoBitrateMode,
    VideoBitratePreset,
    MicrophoneControllerState,
    MicrophonePreferenceMode,
    MicrophoneDisconnectPolicy,
    MicrophoneAutoDisconnectTransitionFailedEvent,
  } from "$lib/types";

  // ─── State ────────────────────────────────────────────────────────────────

  let captureSupport = $state<CaptureSupport | null>(null);
  let recordingSettings = $state<RecordingSettings | null>(null);
  let micState = $state<MicrophoneControllerState | null>(null);

  // Recording settings drafts
  let draftCaptureScreen = $state(true);
  let draftCaptureMicrophone = $state(false);
  let draftCaptureSystemAudio = $state(false);
  let draftSegmentDuration = $state(60);
  let draftFrameRate = $state(30);
  let draftSaveDirectory = $state("");
  let draftAutoStart = $state(false);

  // Resolution drafts
  let draftResolutionMode = $state<ResolutionMode>("original");
  let draftResolutionPreset = $state<ResolutionPreset>("1080p");
  let draftCustomWidth = $state<number | null>(null);
  let draftCustomHeight = $state<number | null>(null);
  let customWidthRaw = $state("");
  let customHeightRaw = $state("");

  // Video bitrate drafts
  let draftBitrateMode = $state<VideoBitrateMode>("preset");
  let draftBitratePreset = $state<VideoBitratePreset>("medium");
  let draftCustomMbpsRaw = $state("");
  let draftCustomMbps = $state<number | null>(null);

  // Microphone drafts
  let draftPreferenceMode = $state<MicrophonePreferenceMode>("default");
  let draftDeviceId = $state<string | null>(null);
  let draftDisconnectPolicy = $state<MicrophoneDisconnectPolicy>("fallback_to_default");

  // Inactivity drafts
  let draftPauseCaptureOnInactivity = $state(false);
  let draftIdleTimeoutSeconds = $state(30);
  let draftActivityMode = $state<ActivityMode>("system_input_only");
  let draftMicrophoneActivitySensitivity = $state(50);
  let draftSystemAudioActivitySensitivity = $state(50);

  // Debug logging draft
  let draftNativeCaptureDebugLoggingEnabled = $state(false);

  // Debug log status
  let debugLogStatus = $state<NativeCaptureDebugLogStatus | null>(null);
  let loadingDebugLogStatus = $state(false);
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

  // Loading / error state
  let loadingRecSettings = $state(false);
  let savingRecSettings = $state(false);
  let loadingMicState = $state(false);
  let savingMicSettings = $state(false);
  let recError = $state<string | null>(null);
  let micError = $state<string | null>(null);
  let recSaved = $state(false);
  let micSaved = $state(false);

  // Capture-support fetch lifecycle: tracks whether the in-flight request
  // is still running and whether it ended in an unrecoverable failure.
  let captureSupportLoading = $state(false);
  let captureSupportFailed = $state(false);

  // ─── Backend capability ────────────────────────────────────────────────────
  const nativeCaptureUnsupported = $derived(
    captureSupport !== null && !captureSupport.nativeCaptureSupported
  );

  // The AVFoundation fallback backend (pre-macOS 15) only supports "original"
  // resolution. ScreenCaptureKit (macOS 15+) supports all modes.
  // The same macOS version gate controls both system audio and the SCKit
  // backend, so `supportedSources.systemAudio === false` is a precise proxy.
  const onlyOriginalResolutionSupported = $derived(
    captureSupport !== null
    && captureSupport.nativeCaptureSupported
    && !captureSupport.supportedSources.systemAudio
  );

  const nonOriginalResolutionSupported = $derived(
    captureSupport !== null
    && captureSupport.nativeCaptureSupported
    && captureSupport.supportedSources.systemAudio
  );

  // True ONLY while the support request is actively in-flight.
  // A failed lookup is NOT treated as pending — the request has completed,
  // just without useful data. Keeping it "pending" forever would permanently
  // block the user, so we distinguish the two states explicitly.
  const resolutionSupportPending = $derived(captureSupportLoading);

  // Preset and custom are selectable only once support is confirmed available.
  // Three cases: (1) in-flight → disabled, (2) loaded but AVFoundation-only →
  // disabled, (3) loaded with SCKit OR lookup failed → enabled (backend
  // validates at save time if we could not determine support locally).
  const nonOriginalResolutionDisabled = $derived(
    draftCaptureScreen
    && (resolutionSupportPending || nativeCaptureUnsupported || onlyOriginalResolutionSupported)
  );

  // Block saving only while the request is genuinely in-flight for non-original
  // modes. A failed lookup unblocks saving so the backend can validate instead.
  const resolutionSupportPendingForNonOriginal = $derived(
    draftCaptureScreen && resolutionSupportPending && draftResolutionMode !== "original"
  );

  // ─── Helpers ──────────────────────────────────────────────────────────────

  function syncRecDrafts(s: RecordingSettings) {
    draftCaptureScreen = s.captureScreen;
    draftCaptureMicrophone = s.captureMicrophone;
    draftCaptureSystemAudio = s.captureSystemAudio;
    draftSegmentDuration = s.segmentDurationSeconds;
    draftFrameRate = s.screenFrameRate;
    draftSaveDirectory = s.saveDirectory;
    draftAutoStart = s.autoStart;
    draftPauseCaptureOnInactivity = s.pauseCaptureOnInactivity;
    draftIdleTimeoutSeconds = s.idleTimeoutSeconds;
    draftActivityMode = s.activityMode ?? "system_input_only";
    draftMicrophoneActivitySensitivity = s.microphoneActivitySensitivity ?? 50;
    draftSystemAudioActivitySensitivity = s.systemAudioActivitySensitivity ?? 50;
    draftNativeCaptureDebugLoggingEnabled = s.nativeCaptureDebugLoggingEnabled ?? false;
    if (s.screenResolution.mode === "custom") {
      draftResolutionMode = "custom";
      draftCustomWidth = s.screenResolution.width;
      draftCustomHeight = s.screenResolution.height;
      customWidthRaw = String(s.screenResolution.width);
      customHeightRaw = String(s.screenResolution.height);
    } else if (s.screenResolution.preset === "original") {
      draftResolutionMode = "original";
      draftResolutionPreset = "1080p";
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    } else {
      draftResolutionMode = "preset";
      draftResolutionPreset = s.screenResolution.preset;
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    }
    // Video bitrate
    if (s.videoBitrate.mode === "custom") {
      draftBitrateMode = "custom";
      draftBitratePreset = "medium";
      draftCustomMbps = s.videoBitrate.customMbps;
      draftCustomMbpsRaw = String(s.videoBitrate.customMbps);
    } else {
      draftBitrateMode = "preset";
      draftBitratePreset = s.videoBitrate.preset;
      draftCustomMbps = null;
      draftCustomMbpsRaw = "";
    }
  }

  function syncMicDrafts(s: MicrophoneControllerState) {
    draftPreferenceMode = s.preference.mode;
    draftDeviceId = s.preference.deviceId ?? null;
    draftDisconnectPolicy = s.disconnectPolicy;
  }

  // ─── Actions ──────────────────────────────────────────────────────────────

  async function loadCaptureSupport() {
    captureSupportLoading = true;
    captureSupportFailed = false;
    // Clear any stale data immediately so derived state (nativeCaptureUnsupported,
    // onlyOriginalResolutionSupported, nonOriginalResolutionSupported) reflects
    // the in-flight state rather than a previous result while the request runs.
    captureSupport = null;
    try {
      captureSupport = await invoke<CaptureSupport>("get_capture_support");
    } catch {
      // Non-fatal: support info is best-effort. Mark as failed so the UI can
      // distinguish "still loading" from "lookup failed". Preset/custom options
      // are unblocked on failure and the backend validates the selection on save.
      // captureSupport stays null here, which is correct — all capability-gated
      // derived values require captureSupport !== null, so none will fire.
      captureSupportFailed = true;
    } finally {
      captureSupportLoading = false;
    }
  }

  async function loadDebugLogStatus() {
    loadingDebugLogStatus = true;
    debugLogError = null;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>("get_native_capture_debug_log_status");
    } catch (err) {
      debugLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingDebugLogStatus = false;
    }
  }

  async function deleteDebugLog() {
    deletingDebugLog = true;
    debugLogError = null;
    debugLogDeleted = false;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>("delete_native_capture_debug_log");
      debugLogDeleted = true;
      setTimeout(() => { debugLogDeleted = false; }, 2200);
    } catch (err) {
      debugLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      openingGeneralLog = false;
    }
  }

  async function deleteGeneralLog() {
    deletingGeneralLog = true;
    generalLogError = null;
    generalLogDeleted = false;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("delete_general_app_log");
      generalLogDeleted = true;
      setTimeout(() => { generalLogDeleted = false; }, 2200);
    } catch (err) {
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      deletingGeneralLog = false;
    }
  }

  async function loadRecordingSettings() {
    loadingRecSettings = true;
    recError = null;
    try {
      const s = await invoke<RecordingSettings>("get_recording_settings");
      recordingSettings = s;
      syncRecDrafts(s);
    } catch (err) {
      recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingRecSettings = false;
    }
  }

  async function saveRecordingSettings() {
    if (resolutionSupportPendingForNonOriginal) {
      recError = "Wait for capture support to load before saving preset/custom resolution.";
      return;
    }

    savingRecSettings = true;
    recError = null;
    recSaved = false;
    try {
      const updated = await invoke<RecordingSettings>("update_recording_settings", {
        request: {
          captureScreen: draftCaptureScreen,
          captureMicrophone: draftCaptureMicrophone,
          captureSystemAudio: draftCaptureSystemAudio,
          segmentDurationSeconds: draftSegmentDuration,
          screenFrameRate: draftFrameRate,
          saveDirectory: draftSaveDirectory,
          autoStart: draftAutoStart,
          pauseCaptureOnInactivity: draftPauseCaptureOnInactivity,
          idleTimeoutSeconds: draftIdleTimeoutSeconds,
          activityMode: draftActivityMode,
          microphoneActivitySensitivity: draftMicrophoneActivitySensitivity,
          systemAudioActivitySensitivity: draftSystemAudioActivitySensitivity,
          nativeCaptureDebugLoggingEnabled: draftNativeCaptureDebugLoggingEnabled,
          screenResolution: draftResolutionMode === "custom"
            ? {
                mode: "custom",
                width: draftCustomWidth!,
                height: draftCustomHeight!,
              }
            : {
                mode: "preset",
                preset: draftResolutionMode === "original" ? "original" : draftResolutionPreset,
              },
          videoBitrate: draftBitrateMode === "custom"
            ? { mode: "custom", preset: null, customMbps: draftCustomMbps! }
            : { mode: "preset", preset: draftBitratePreset, customMbps: null },
        },
      });
      recordingSettings = updated;
      syncRecDrafts(updated);
      recSaved = true;
      setTimeout(() => { recSaved = false; }, 2200);
      // Refresh debug log status since the enabled flag may have changed.
      loadDebugLogStatus();
    } catch (err) {
      recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      savingRecSettings = false;
    }
  }

  async function loadMicState() {
    loadingMicState = true;
    micError = null;
    try {
      const s = await invoke<MicrophoneControllerState>("get_microphone_controller_state");
      micState = s;
      syncMicDrafts(s);
    } catch (err) {
      micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingMicState = false;
    }
  }

  async function saveMicSettings() {
    savingMicSettings = true;
    micError = null;
    micSaved = false;
    try {
      const updated = await invoke<MicrophoneControllerState>("update_microphone_controller", {
        request: {
          preference: {
            mode: draftPreferenceMode,
            deviceId: draftPreferenceMode === "specific_device" ? draftDeviceId : null,
          },
          disconnectPolicy: draftDisconnectPolicy,
        },
      });
      micState = updated;
      syncMicDrafts(updated);
      micSaved = true;
      setTimeout(() => { micSaved = false; }, 2200);
    } catch (err) {
      micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      savingMicSettings = false;
    }
  }

  // ─── Recording settings validation ───────────────────────────────────────

  // Invariant: system audio requires screen capture.
  // Reactively coerce the draft: if screen is turned off, force system audio off too.
  $effect(() => {
    if (!draftCaptureScreen && draftCaptureSystemAudio) {
      draftCaptureSystemAudio = false;
    }
  });

  // Invariant: coerce any non-original draft back to "original" only once we
  // have confirmed that non-original is unsupported (AVFoundation / pre-macOS 15).
  // While support is still pending we preserve the loaded draft intact — the
  // UI disables the radio options so the user cannot change them, and saving is
  // blocked by resolutionSupportPendingForNonOriginal, but we must not destroy
  // a valid preset/custom draft that was loaded from persisted settings.
  $effect(() => {
    if (draftCaptureScreen && onlyOriginalResolutionSupported && draftResolutionMode !== "original") {
      draftResolutionMode = "original";
    }
  });

  function parseCustomDimension(raw: string): number | null {
    if (!/^\d+$/.test(raw)) return null;
    const value = Number(raw);
    if (!Number.isInteger(value)) return null;
    return value;
  }

  // Parse custom resolution inputs as integers; keep null if invalid.
  $effect(() => {
    const w = parseCustomDimension(customWidthRaw);
    draftCustomWidth = w ?? null;
  });
  $effect(() => {
    const h = parseCustomDimension(customHeightRaw);
    draftCustomHeight = h ?? null;
  });

  // Parse custom bitrate input as an integer (Mbps); keep null if invalid.
  $effect(() => {
    if (!draftCustomMbpsRaw) { draftCustomMbps = null; return; }
    if (!/^\d+$/.test(draftCustomMbpsRaw.trim())) { draftCustomMbps = null; return; }
    const val = parseInt(draftCustomMbpsRaw.trim(), 10);
    draftCustomMbps = Number.isInteger(val) && val > 0 ? val : null;
  });

  const customResolutionErrors = $derived((() => {
    if (draftResolutionMode !== "custom") return [];
    const errors: string[] = [];
    const w = parseCustomDimension(customWidthRaw);
    const h = parseCustomDimension(customHeightRaw);
    if (customWidthRaw && w === null) errors.push("Width must be an integer.");
    if (customHeightRaw && h === null) errors.push("Height must be an integer.");
    if (w != null && (w < 16 || w > 8192)) errors.push("Width must be between 16 and 8192.");
    if (h != null && (h < 16 || h > 8192)) errors.push("Height must be between 16 and 8192.");
    if (!customWidthRaw || !customHeightRaw) errors.push("Both width and height are required for custom mode.");
    return errors;
  })());

  const customResolutionBlocked = $derived(
    draftResolutionMode === "custom" && customResolutionErrors.length > 0
  );

  const customBitrateErrors = $derived((() => {
    if (draftBitrateMode !== "custom") return [];
    const errors: string[] = [];
    if (!draftCustomMbpsRaw) {
      errors.push("Custom bitrate is required (1–40 Mbps, whole number).");
    } else if (!/^\d+$/.test(draftCustomMbpsRaw.trim())) {
      errors.push("Bitrate must be a whole number of Mbps (e.g. 12).");
    } else {
      const val = parseInt(draftCustomMbpsRaw.trim(), 10);
      if (!Number.isInteger(val) || val <= 0) {
        errors.push("Bitrate must be a positive whole number.");
      } else if (val < 1) {
        errors.push("Bitrate must be at least 1 Mbps.");
      } else if (val > 40) {
        errors.push("Bitrate must not exceed 40 Mbps.");
      }
    }
    return errors;
  })());

  const customBitrateBlocked = $derived(
    draftBitrateMode === "custom" && customBitrateErrors.length > 0
  );

  const recValidationErrors = $derived((() => {
    const errors: string[] = [];
    const anySource = draftCaptureScreen || draftCaptureMicrophone || draftCaptureSystemAudio;
    if (!anySource) {
      errors.push("At least one capture source (Screen, Microphone, or System Audio) must be enabled.");
    }
    if (draftCaptureSystemAudio && !draftCaptureScreen) {
      errors.push("System Audio capture requires Screen capture to be enabled.");
    }
    if (resolutionSupportPendingForNonOriginal) {
      errors.push("Wait for capture support to load before saving preset/custom resolution.");
    }
    return errors;
  })());

  const recSaveBlocked = $derived(
    recValidationErrors.length > 0 || !draftSaveDirectory || customResolutionBlocked || customBitrateBlocked
  );

  const micApplyBlocked = $derived(
    draftPreferenceMode === "specific_device" && !draftDeviceId
  );

  const micDeviceOptions = $derived(
    (micState?.devices ?? []).map((d) => ({
      value: d.id,
      label: d.name + (d.isDefault ? " (default)" : ""),
    }))
  );

  // ─── Init ─────────────────────────────────────────────────────────────────

  $effect(() => {
    loadCaptureSupport();
    loadRecordingSettings();
    loadMicState();
    loadDebugLogStatus();
    loadGeneralLogStatus();

    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let destroyed = false;

    listen<MicrophoneControllerState>("microphone_controller_changed", (event) => {
      micState = event.payload;
      syncMicDrafts(event.payload);
      micError = null;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenControllerChanged = fn;
    });

    listen<MicrophoneAutoDisconnectTransitionFailedEvent>(
      "microphone_auto_disconnect_transition_failed",
      (event) => {
        const { context, code, message } = event.payload;
        micError = `[${context}] [${code}] ${message}`;
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenAutoDisconnectFailure = fn;
    });

    return () => {
      destroyed = true;
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
    };
  });
</script>

<!-- ── Page header ──────────────────────────────────────────────────────── -->
<div class="page-header">
  <h1 class="page-title">Settings</h1>
  <p class="page-subtitle">recording &amp; microphone configuration</p>
</div>

<!-- ── Recording settings ───────────────────────────────────────────────── -->
<section class="card">
  <div class="card__header">
    <h2 class="card__title">Recording</h2>
    <button class="btn btn--ghost btn--sm" onclick={loadRecordingSettings} disabled={loadingRecSettings}>
      {loadingRecSettings ? "…" : "Reload"}
    </button>
  </div>

  {#if loadingRecSettings}
    <p class="loading-text">Loading settings…</p>
  {:else}
    <div class="settings-group">
      <span class="group-label">Capture Sources</span>
      <div class="settings-stack">
        <Switch
          bind:checked={draftCaptureScreen}
          label="Screen"
          description="Capture the display"
        />
        <Switch
          bind:checked={draftCaptureMicrophone}
          label="Microphone"
          description="Capture audio from microphone"
        />
        <Switch
          bind:checked={draftCaptureSystemAudio}
          disabled={!draftCaptureScreen}
          label="System Audio"
          description="Capture Mac system audio (macOS 15+)"
        />
        {#if !draftCaptureScreen}
          <p class="capture-source-hint">System Audio is unavailable — enable Screen first.</p>
        {/if}
      </div>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Segment Duration</span>
      <Slider
        bind:value={draftSegmentDuration}
        min={10}
        max={600}
        step={10}
        label="Duration"
        unit="s"
        formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60}s` : `${v}s`}
      />
      <p class="group-hint">How long each recording segment is before a new one starts.</p>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Screen Frame Rate</span>
      <Slider
        bind:value={draftFrameRate}
        min={1}
        max={120}
        step={1}
        label="Frame rate"
        unit=" fps"
      />
      <p class="group-hint">Higher frame rates produce larger files.</p>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Screen Resolution</span>

      {#if nativeCaptureUnsupported}
        <div class="resolution-unsupported-notice">
          <span class="resolution-unsupported-notice__icon">ℹ</span>
          <span class="resolution-unsupported-notice__text">
            Native screen capture is unsupported on this system. Resolution settings are saved,
            but only apply when native screen capture is available.
          </span>
        </div>
      {:else if onlyOriginalResolutionSupported}
        <div class="resolution-locked-notice">
          <span class="resolution-locked-notice__icon">ℹ</span>
          <span class="resolution-locked-notice__text">
            Preset and custom resolutions require macOS 15 or later (ScreenCaptureKit).
            Only <strong>Original</strong> resolution is available on this system.
          </span>
        </div>
      {:else if resolutionSupportPending}
        <div class="resolution-pending-notice">
          <span class="resolution-pending-notice__icon">⏳</span>
          <span class="resolution-pending-notice__text">
            Checking capture support… Preset and Custom are disabled until support is confirmed.
          </span>
        </div>
      {:else if captureSupportFailed}
        <div class="resolution-warn-notice">
          <span class="resolution-warn-notice__icon">⚠</span>
          <span class="resolution-warn-notice__text">
            Could not determine capture support for this system. You can still edit and save —
            the backend will validate the chosen resolution.
          </span>
        </div>
      {:else if nonOriginalResolutionSupported}
        <div class="resolution-supported-notice">
          <span class="resolution-supported-notice__icon">✓</span>
          <span class="resolution-supported-notice__text">
            Native capture supports Preset and Custom output resolutions.
          </span>
        </div>
      {/if}

      <RadioGroup
        bind:value={draftResolutionMode}
        disabledValues={nonOriginalResolutionDisabled ? ["preset", "custom"] : []}
        options={[
          { value: "original", label: "Original", description: "Capture at the display's native resolution" },
          { value: "preset", label: "Preset", description: "Select a standard output resolution" },
          { value: "custom", label: "Custom", description: "Enter exact width and height in pixels" },
        ]}
      />

      {#if draftResolutionMode === "preset"}
        <div class="resolution-preset-grid">
          {#each (["1080p", "720p", "540p"] as const) as preset}
            {@const presetMeta = { "1080p": { w: 1920, h: 1080 }, "720p": { w: 1280, h: 720 }, "540p": { w: 960, h: 540 } }[preset]}
            <button
              class="preset-chip"
              class:preset-chip--active={draftResolutionPreset === preset}
              onclick={() => { draftResolutionPreset = preset; }}
              type="button"
            >
              <span class="preset-chip__label">{preset}</span>
              <span class="preset-chip__dim">{presetMeta.w}×{presetMeta.h}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if draftResolutionMode === "custom"}
        <div class="custom-resolution-inputs">
          <div class="custom-res-field">
            <label class="custom-res-label" for="res-width">Width (px)</label>
            <input
              id="res-width"
              type="text"
              inputmode="numeric"
              class="text-input custom-res-input"
              class:text-input--empty={customWidthRaw && draftCustomWidth === null}
              bind:value={customWidthRaw}
              placeholder="e.g. 1920"
              autocomplete="off"
            />
          </div>
          <span class="custom-res-sep" aria-hidden="true">×</span>
          <div class="custom-res-field">
            <label class="custom-res-label" for="res-height">Height (px)</label>
            <input
              id="res-height"
              type="text"
              inputmode="numeric"
              class="text-input custom-res-input"
              class:text-input--empty={customHeightRaw && draftCustomHeight === null}
              bind:value={customHeightRaw}
              placeholder="e.g. 1080"
              autocomplete="off"
            />
          </div>
        </div>

        {#if customResolutionErrors.length > 0}
          <div class="inline-validation">
            {#each customResolutionErrors as err}
              <p class="inline-validation__item">
                <span class="inline-validation__icon">⚠</span>
                {err}
              </p>
            {/each}
          </div>
        {/if}
      {/if}

      <p class="group-hint">
        {#if draftResolutionMode === "original"}
          Output files will match the captured display's native pixel dimensions.
        {:else if draftResolutionMode === "preset"}
          Output will be scaled to the selected preset. Aspect ratio is preserved.
        {:else}
          Output will be scaled to the exact dimensions you specify.
        {/if}
      </p>
    </div>

    <div class="settings-divider"></div>

    <!-- ── Video Bitrate ──────────────────────────────────────── -->
    <div class="settings-group">
      <span class="group-label">Video Bitrate</span>
      <p class="group-hint">
        Bitrate controls the amount of data encoded per second of video.
        Higher bitrate = sharper image and less compression artefact, but
        larger files and higher CPU/GPU load. Lower bitrate reduces file size
        and power use at the cost of some visual quality.
        This setting is applied on <strong>macOS 15+ via ScreenCaptureKit</strong>;
        older systems fall back to the macOS system-default bitrate.
      </p>

      <!-- Mode selector (preset chips + custom) -->
      <div class="bitrate-mode-chips">
        {#each (["low", "medium", "high"] as const) as bp}
          {@const meta = { low: { mbps: "~3", hint: "Lower quality, smallest file" }, medium: { mbps: "~8", hint: "Balanced quality and size" }, high: { mbps: "~20", hint: "High quality, larger file" } }[bp]}
          <button
            type="button"
            class="bitrate-chip"
            class:bitrate-chip--active={draftBitrateMode === "preset" && draftBitratePreset === bp}
            onclick={() => { draftBitrateMode = "preset"; draftBitratePreset = bp; }}
          >
            <span class="bitrate-chip__label">{bp}</span>
            <span class="bitrate-chip__mbps">{meta.mbps} Mbps</span>
          </button>
        {/each}
        <button
          type="button"
          class="bitrate-chip"
          class:bitrate-chip--active={draftBitrateMode === "custom"}
          onclick={() => { draftBitrateMode = "custom"; }}
        >
          <span class="bitrate-chip__label">Custom</span>
          <span class="bitrate-chip__mbps">1–40 Mbps (integer)</span>
        </button>
      </div>

      {#if draftBitrateMode === "preset"}
        <p class="group-hint bitrate-preset-hint">
          {#if draftBitratePreset === "low"}
            <strong>Low</strong> — ~3 Mbps. Good for long sessions, minimal storage. Best for
            low-motion content or when disk space is limited.
          {:else if draftBitratePreset === "medium"}
            <strong>Medium</strong> — ~8 Mbps. Recommended default. Balanced quality and file
            size for most screen recordings.
          {:else}
            <strong>High</strong> — ~20 Mbps. Crisp detail and smooth motion at the cost of
            larger files. Ideal for high-motion content or final delivery.
          {/if}
          {#if draftFrameRate && draftResolutionMode !== "custom"}
            {' '}At {draftFrameRate} fps{draftResolutionMode === "preset" ? ` / ${draftResolutionPreset}` : draftResolutionMode === "original" ? " / original resolution" : ""}.
          {/if}
        </p>
      {/if}

      {#if draftBitrateMode === "custom"}
        <div class="custom-bitrate-row">
          <div class="custom-res-field">
            <label class="custom-res-label" for="bitrate-mbps">Bitrate (Mbps, whole number)</label>
            <div class="custom-bitrate-input-wrap">
              <input
                id="bitrate-mbps"
                type="text"
                inputmode="numeric"
                class="text-input custom-bitrate-input"
                class:text-input--empty={draftCustomMbpsRaw && draftCustomMbps === null}
                bind:value={draftCustomMbpsRaw}
                placeholder="e.g. 12"
                autocomplete="off"
              />
              <span class="custom-bitrate-unit">Mbps</span>
            </div>
          </div>
        </div>

        {#if customBitrateErrors.length > 0}
          <div class="inline-validation">
            {#each customBitrateErrors as err}
              <p class="inline-validation__item">
                <span class="inline-validation__icon">⚠</span>
                {err}
              </p>
            {/each}
          </div>
        {:else if draftCustomMbps !== null}
          <p class="group-hint">
            Custom bitrate: <strong>{draftCustomMbps} Mbps</strong>.
            {#if draftCustomMbps < 3}
              Low quality — may show compression artefacts on fast-moving content.
            {:else if draftCustomMbps <= 12}
              Moderate quality — good for most recordings.
            {:else if draftCustomMbps <= 25}
              High quality — suitable for detail-sensitive content.
            {:else}
              Very high bitrate — expect large output files.
            {/if}
            {#if draftFrameRate && draftResolutionMode !== "custom"}
              At {draftFrameRate} fps{draftResolutionMode === "preset" ? ` / ${draftResolutionPreset}` : draftResolutionMode === "original" ? " / original resolution" : ""}.
            {/if}
          </p>
        {/if}
      {/if}

      <div class="bitrate-compat-notice">
        <span class="bitrate-compat-notice__icon">ℹ</span>
        <span class="bitrate-compat-notice__text">
          Bitrate is applied only on macOS 15+ (ScreenCaptureKit path).
          On older macOS the system default bitrate is used regardless of this setting.
        </span>
      </div>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Save Directory</span>
      <div class="input-row">
        <input
          type="text"
          class="text-input"
          class:text-input--empty={!draftSaveDirectory}
          bind:value={draftSaveDirectory}
          placeholder="/path/to/recordings"
        />
      </div>
      <p class="group-hint">Where capture files are saved on disk.</p>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Startup</span>
      <Switch
        bind:checked={draftAutoStart}
        label="Auto-start recording on launch"
        description="Begin capturing immediately when the app opens"
      />
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Inactivity Pause</span>
      <Switch
        bind:checked={draftPauseCaptureOnInactivity}
        label="Pause capture when idle"
        description="Automatically pause recording after the system has been idle, and resume when system activity is detected"
      />
      {#if draftPauseCaptureOnInactivity}
        <div class="idle-timeout-row">
          <Slider
            bind:value={draftIdleTimeoutSeconds}
            min={5}
            max={300}
            step={5}
            label="Idle timeout"
            unit="s"
            formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60 > 0 ? ` ${v%60}s` : ""}`.trim() : `${v}s`}
          />
        </div>
        <p class="group-hint">
          Capture pauses after <strong>{draftIdleTimeoutSeconds}s</strong> of system-wide inactivity (no mouse, keyboard,
          or other input anywhere on the Mac). It resumes automatically when system activity is detected again.
        </p>

        <div class="settings-divider"></div>

        <span class="group-label">Activity Mode</span>
        <RadioGroup
          bind:value={draftActivityMode}
          options={[
            {
              value: "system_input_only",
              label: "Input only",
              description: "Only keyboard and mouse/pointer events count as activity. Recording pauses whenever direct input stops, even during video calls or media playback.",
            },
            {
              value: "system_input_or_screen",
              label: "Input or screen change",
              description: "Keyboard/mouse input AND visible on-screen changes (video calls, animations, media) both count as activity. Helps keep recordings running during calls or video playback with no direct input.",
            },
            {
              value: "system_input_or_screen_or_audio",
              label: "Input, screen, or audio",
              description: "All of the above, plus microphone and system audio levels. Sound picked up by the microphone or played through the system keeps capture active — useful for meetings, voice sessions, or any audio-driven workflow.",
            },
          ]}
        />
        <p class="group-hint">
          {#if draftActivityMode === "system_input_or_screen_or_audio"}
            <strong>Audio mode</strong> monitors keyboard/mouse, on-screen changes, <em>and</em>
            audio levels from both the microphone and system audio. Any sound above the configured
            sensitivity threshold counts as activity and keeps the recording running.
          {:else if draftActivityMode === "system_input_or_screen"}
            <strong>Screen change mode</strong> monitors on-screen activity in addition to input events — useful for
            keeping recordings active during video calls, live streams, or media playback where you may not be
            typing or moving the mouse.
          {:else}
            <strong>Input-only mode</strong> triggers the idle timeout strictly on keyboard and mouse inactivity.
            Suitable for general screen recording when you want pauses to match direct interaction gaps exactly.
          {/if}
        </p>

        {#if draftActivityMode === "system_input_or_screen_or_audio"}
          <div class="settings-divider"></div>
          <span class="group-label">Microphone Activity Sensitivity</span>
          <Slider
            bind:value={draftMicrophoneActivitySensitivity}
            min={0}
            max={100}
            step={1}
            label="Mic sensitivity"
            unit="%"
            disabled={!draftCaptureMicrophone}
          />
          {#if !draftCaptureMicrophone}
            <p class="group-hint group-hint--warn">Microphone capture is disabled — this setting has no effect until enabled.</p>
          {:else}
            <p class="group-hint">
              {#if draftMicrophoneActivitySensitivity >= 80}
                <strong>Very high</strong> — whispers and background noise keep capture active.
              {:else if draftMicrophoneActivitySensitivity >= 60}
                <strong>High</strong> — quiet speech counts as activity.
              {:else if draftMicrophoneActivitySensitivity >= 40}
                <strong>Medium</strong> — normal speech triggers activity. Recommended.
              {:else if draftMicrophoneActivitySensitivity >= 20}
                <strong>Low</strong> — only louder audio keeps capture active.
              {:else}
                <strong>Very low</strong> — only very loud audio triggers activity.
              {/if}
            </p>
          {/if}

          <div class="settings-divider"></div>
          <span class="group-label">System Audio Activity Sensitivity</span>
          <Slider
            bind:value={draftSystemAudioActivitySensitivity}
            min={0}
            max={100}
            step={1}
            label="System audio sensitivity"
            unit="%"
            disabled={!draftCaptureSystemAudio}
          />
          {#if !draftCaptureSystemAudio}
            <p class="group-hint group-hint--warn">System audio capture is disabled — this setting has no effect until enabled.</p>
          {:else}
            <p class="group-hint">
              {#if draftSystemAudioActivitySensitivity >= 80}
                <strong>Very high</strong> — quiet system sounds keep capture active.
              {:else if draftSystemAudioActivitySensitivity >= 60}
                <strong>High</strong> — moderate system audio counts as activity.
              {:else if draftSystemAudioActivitySensitivity >= 40}
                <strong>Medium</strong> — typical media playback triggers activity. Recommended.
              {:else if draftSystemAudioActivitySensitivity >= 20}
                <strong>Low</strong> — only louder system audio keeps capture active.
              {:else}
                <strong>Very low</strong> — only very loud system audio triggers activity.
              {/if}
            </p>
          {/if}

          <div class="audio-activity-notice">
            <span class="audio-activity-notice__icon">♪</span>
            <span class="audio-activity-notice__text">
              {#if !draftCaptureMicrophone && !draftCaptureSystemAudio}
                Neither microphone nor system audio capture is enabled — audio activity detection
                will not function. Enable at least one source in <strong>Capture Sources</strong> above.
              {:else if !draftCaptureMicrophone}
                Microphone capture is disabled — only system audio is monitored for activity.
              {:else if !draftCaptureSystemAudio}
                System audio capture is disabled — only microphone audio is monitored for activity.
              {:else}
                Both microphone and system audio are monitored independently for activity.
              {/if}
            </span>
          </div>
        {/if}
      {/if}
    </div>

    <div class="settings-divider"></div>

    <!-- ── Native Capture Debug Logging ──────────────────────── -->
    <div class="settings-group">
      <span class="group-label">Native Capture Debug Logging</span>
      <Switch
        bind:checked={draftNativeCaptureDebugLoggingEnabled}
        label="Enable debug logging"
        description="Write native capture diagnostic output to a log file on disk"
      />
      <p class="group-hint">
        When enabled, native capture internals are logged to a file for troubleshooting.
        Save settings to apply the change.
      </p>

      {#if debugLogStatus}
        <div class="debug-log-status">
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Status</span>
            <span class="debug-log-status__value">
              {#if debugLogStatus.enabled}
                <span class="debug-log-status__dot debug-log-status__dot--on"></span> Active
              {:else}
                <span class="debug-log-status__dot"></span> Inactive
              {/if}
            </span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Path</span>
            <span class="debug-log-status__path" title={debugLogStatus.path}>{debugLogStatus.path}</span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">File</span>
            <span class="debug-log-status__value">{debugLogStatus.exists ? "Exists on disk" : "Not found"}</span>
          </div>
        </div>

        {#if debugLogStatus.exists}
          <div class="debug-log-actions">
            <button
              class="btn btn--danger btn--sm"
              onclick={deleteDebugLog}
              disabled={deletingDebugLog}
            >
              {deletingDebugLog ? "Deleting…" : "Delete Log File"}
            </button>
            {#if debugLogDeleted}
              <span class="saved-badge">✓ Deleted</span>
            {/if}
          </div>
        {/if}
      {:else if loadingDebugLogStatus}
        <p class="loading-text">Loading log status…</p>
      {/if}

      {#if debugLogError}
        <div class="inline-error">
          <span class="inline-error__icon">⚠</span>
          <span class="inline-error__msg">{debugLogError}</span>
          <button class="btn btn--ghost btn--sm" onclick={() => debugLogError = null}>×</button>
        </div>
      {/if}
    </div>

    <div class="settings-divider"></div>

    <!-- ── General Application Log ───────────────────────────── -->
    <div class="settings-group">
      <span class="group-label">General Application Log</span>
      <p class="group-hint">
        The general application log captures high-level runtime events and errors.
      </p>

      {#if generalLogStatus}
        <div class="debug-log-status">
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Path</span>
            <span class="debug-log-status__path" title={generalLogStatus.path}>{generalLogStatus.path}</span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">File</span>
            <span class="debug-log-status__value">{generalLogStatus.exists ? "Exists on disk" : "Not found"}</span>
          </div>
        </div>

        <div class="debug-log-actions">
          <button
            class="btn btn--ghost btn--sm"
            onclick={openGeneralLog}
            disabled={openingGeneralLog}
          >
            {#if openingGeneralLog}
              Opening…
            {:else if generalLogStatus.exists}
              Open Log File
            {:else}
              Open Containing Folder
            {/if}
          </button>
          {#if generalLogStatus.exists}
            <button
              class="btn btn--danger btn--sm"
              onclick={deleteGeneralLog}
              disabled={deletingGeneralLog}
            >
              {deletingGeneralLog ? "Deleting…" : "Delete Log File"}
            </button>
          {/if}
          {#if generalLogDeleted}
            <span class="saved-badge">✓ Deleted</span>
          {/if}
        </div>
      {:else if loadingGeneralLogStatus}
        <p class="loading-text">Loading log status…</p>
      {/if}

      {#if generalLogError}
        <div class="inline-error">
          <span class="inline-error__icon">⚠</span>
          <span class="inline-error__msg">{generalLogError}</span>
          <button class="btn btn--ghost btn--sm" onclick={() => generalLogError = null}>×</button>
        </div>
      {/if}
    </div>

    {#if recError}
      <div class="inline-error">
        <span class="inline-error__icon">⚠</span>
        <span class="inline-error__msg">{recError}</span>
        <button class="btn btn--ghost btn--sm" onclick={() => recError = null}>×</button>
      </div>
    {/if}

    {#if recValidationErrors.length > 0}
      <div class="inline-validation">
        {#each recValidationErrors as err}
          <p class="inline-validation__item">
            <span class="inline-validation__icon">⚠</span>
            {err}
          </p>
        {/each}
      </div>
    {/if}

    <div class="action-row">
      <button
        class="btn btn--primary"
        onclick={saveRecordingSettings}
        disabled={savingRecSettings || recSaveBlocked}
      >
        {savingRecSettings ? "Saving…" : "Save Recording Settings"}
      </button>
      {#if recSaved}
        <span class="saved-badge">✓ Saved</span>
      {/if}
    </div>
  {/if}
</section>

<!-- ── Microphone settings ───────────────────────────────────────────────── -->
<section class="card">
  <div class="card__header">
    <h2 class="card__title">Microphone Controller</h2>
    <button class="btn btn--ghost btn--sm" onclick={loadMicState} disabled={loadingMicState}>
      {loadingMicState ? "…" : "Reload"}
    </button>
  </div>

  {#if loadingMicState}
    <p class="loading-text">Loading microphone state…</p>
  {:else if micState}
    <!-- Effective device banner -->
    <div class="effective-device" class:effective-device--none={!micState.effectiveDevice}>
      <span class="effective-device__dot" class:effective-device__dot--on={!!micState.effectiveDevice}></span>
      <span class="effective-device__label">
        {#if micState.effectiveDevice}
          {micState.effectiveDevice.name}
          {#if micState.effectiveDevice.isDefault}
            <span class="badge badge--neutral badge--sm">default</span>
          {/if}
        {:else}
          No active device
        {/if}
      </span>
    </div>

    <!-- Available devices -->
    {#if micState.devices.length > 0}
      <div class="settings-group">
        <span class="group-label">Available Devices</span>
        <ul class="device-list">
          {#each micState.devices as device (device.id)}
            <li class="device-item" class:device-item--active={micState.effectiveDevice?.id === device.id}>
              <span class="device-item__dot" class:device-item__dot--active={micState.effectiveDevice?.id === device.id}></span>
              <span class="device-item__name">{device.name}</span>
              <div class="device-item__badges">
                {#if device.isDefault}
                  <span class="badge badge--neutral badge--sm">default</span>
                {/if}
                {#if micState.effectiveDevice?.id === device.id}
                  <span class="badge badge--ok badge--sm">active</span>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {:else}
      <p class="empty-state">No microphone devices found.</p>
    {/if}

    <div class="settings-divider"></div>

    <div class="settings-group">
      <RadioGroup
        bind:value={draftPreferenceMode}
        label="Preference"
        options={[
          { value: "default", label: "System Default", description: "Use the currently selected system microphone" },
          { value: "specific_device", label: "Specific Device", description: "Lock to a particular microphone" },
        ]}
      />
    </div>

    {#if draftPreferenceMode === "specific_device"}
      <div class="settings-group">
        <SelectMenu
          bind:value={draftDeviceId}
          label="Device"
          options={micDeviceOptions}
          placeholder="— pick a device —"
          warn={!draftDeviceId}
        />
        {#if !draftDeviceId}
          <p class="group-hint group-hint--warn">Select a device before saving Specific Device mode.</p>
        {/if}
      </div>
    {/if}

    <div class="settings-divider"></div>

    <div class="settings-group">
      <RadioGroup
        bind:value={draftDisconnectPolicy}
        label="On Disconnect"
        options={[
          { value: "fallback_to_default", label: "Fallback to Default", description: "Switch to system default when device disconnects" },
          { value: "wait_for_same_device", label: "Wait for Same Device", description: "Pause microphone capture until the device reconnects" },
        ]}
      />
    </div>

    {#if micError}
      <div class="inline-error">
        <span class="inline-error__icon">⚠</span>
        <span class="inline-error__msg">{micError}</span>
        <button class="btn btn--ghost btn--sm" onclick={() => micError = null}>×</button>
      </div>
    {/if}

    <div class="action-row">
      <button
        class="btn btn--primary"
        onclick={saveMicSettings}
        disabled={savingMicSettings || micApplyBlocked}
      >
        {savingMicSettings ? "Saving…" : "Save Microphone Settings"}
      </button>
      {#if micSaved}
        <span class="saved-badge">✓ Saved</span>
      {/if}
    </div>
  {:else}
    <p class="empty-state">Failed to load microphone state.</p>
    <button class="btn btn--ghost btn--sm" onclick={loadMicState}>Retry</button>
  {/if}
</section>

<style>
  /* ── Page header ───────────────────────────────────────────── */
  .page-header {
    margin-bottom: 4px;
  }

  .page-title {
    font-size: 18px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #f0f0f5;
  }

  .page-subtitle {
    font-size: 10px;
    color: #44445a;
    letter-spacing: 0.06em;
    margin-top: 2px;
  }

  /* ── Card ──────────────────────────────────────────────────── */
  .card {
    background: #13131a;
    border: 1px solid #1e1e2e;
    border-radius: 6px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .card__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .card__title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #44445a;
  }

  /* ── Settings groups ──────────────────────────────────────── */
  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .group-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #44445a;
  }

  .settings-stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 14px;
    background: #0e0e16;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
  }

  .settings-divider {
    height: 1px;
    background: #1a1a26;
  }

  .group-hint {
    font-size: 10px;
    color: #33334a;
    letter-spacing: 0.03em;
    line-height: 1.5;
  }

  .group-hint--warn {
    color: #8a5020;
    font-weight: 600;
  }

  /* ── Text input ────────────────────────────────────────────── */
  .input-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .text-input {
    flex: 1;
    padding: 7px 10px;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    font-family: inherit;
    font-size: 12px;
    color: #c0c0d0;
    outline: none;
    transition: border-color 0.12s;
  }

  .text-input:focus {
    border-color: #3dffa0;
  }

  .text-input--empty {
    border-color: #7a4a18;
  }

  .text-input::placeholder {
    color: #33334a;
  }

  /* ── Buttons ───────────────────────────────────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn--primary {
    background: #0f2e1f;
    color: #3dffa0;
    border-color: #1a4a30;
  }

  .btn--primary:not(:disabled):hover {
    background: #1a3d2a;
    border-color: #3dffa0;
  }

  .btn--ghost {
    background: transparent;
    color: #7a7a9a;
    border-color: #2a2a3a;
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: #1a1a2a;
    color: #a0a0c0;
    border-color: #3a3a5a;
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  .action-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .saved-badge {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: #3dffa0;
    animation: fade-in-out 2.2s ease forwards;
  }

  @keyframes fade-in-out {
    0% { opacity: 0; transform: translateX(-4px); }
    15% { opacity: 1; transform: translateX(0); }
    80% { opacity: 1; }
    100% { opacity: 0; }
  }

  /* ── Badges ────────────────────────────────────────────────── */
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .badge--ok {
    background: #0f2e1f;
    color: #3dffa0;
    border: 1px solid #1a4a30;
  }

  .badge--neutral {
    background: #1a1a2a;
    color: #7070a0;
    border: 1px solid #2a2a3a;
  }

  .badge--sm {
    padding: 0 5px;
    font-size: 9px;
  }

  /* ── Effective device ─────────────────────────────────────── */
  .effective-device {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: #0a1410;
    border: 1px solid #1a3020;
    border-radius: 5px;
    transition: background 0.2s, border-color 0.2s;
  }

  .effective-device--none {
    background: #0d0d14;
    border-color: #1a1a2a;
  }

  .effective-device__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #33334a;
    flex-shrink: 0;
    transition: background 0.2s;
  }

  .effective-device__dot--on {
    background: #3dffa0;
  }

  .effective-device__label {
    font-size: 12px;
    font-weight: 500;
    color: #9090b0;
    display: flex;
    align-items: center;
    gap: 7px;
  }

  /* ── Device list ───────────────────────────────────────────── */
  .device-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .device-item {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 10px;
    border-radius: 4px;
    background: #0e0e16;
    border: 1px solid #1a1a28;
    transition: border-color 0.12s;
  }

  .device-item--active {
    border-color: #1a3020;
    background: #0a1410;
  }

  .device-item__dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: #2a2a3a;
    flex-shrink: 0;
    transition: background 0.15s;
  }

  .device-item__dot--active {
    background: #3dffa0;
  }

  .device-item__name {
    font-size: 11px;
    color: #9090b0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .device-item__badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  /* ── Inline error ─────────────────────────────────────────── */
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    background: #0e0a0a;
    border: 1px solid #3a1a20;
    border-radius: 4px;
  }

  .inline-error__icon {
    color: #ff6b7a;
    font-size: 11px;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .inline-error__msg {
    font-size: 11px;
    color: #cc5060;
    flex: 1;
    word-break: break-word;
  }

  /* ── Misc ──────────────────────────────────────────────────── */
  .loading-text {
    font-size: 11px;
    color: #33334a;
    font-style: italic;
  }

  .empty-state {
    font-size: 11px;
    color: #2a2a40;
    font-style: italic;
  }

  /* ── Capture source hints ─────────────────────────────────── */
  .capture-source-hint {
    font-size: 10px;
    color: #6a4a1a;
    letter-spacing: 0.03em;
    line-height: 1.5;
    margin-top: 2px;
  }

  /* ── Inline validation ────────────────────────────────────── */
  .inline-validation {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 10px 12px;
    background: #0d0b08;
    border: 1px solid #3a2a10;
    border-radius: 4px;
  }

  .inline-validation__item {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: 11px;
    color: #a06820;
    line-height: 1.5;
  }

  .inline-validation__icon {
    font-size: 10px;
    flex-shrink: 0;
    color: #c07820;
  }

  /* ── Resolution preset chips ──────────────────────────────────────── */
  .resolution-preset-grid {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .preset-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 16px;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    font-family: inherit;
    transition: background 0.12s, border-color 0.12s;
    min-width: 72px;
  }

  .preset-chip:hover {
    background: #131320;
    border-color: #3a3a5a;
  }

  .preset-chip--active {
    background: #0d1f15;
    border-color: #3dffa0;
  }

  .preset-chip:focus-visible {
    outline: 1px solid #3dffa0;
    outline-offset: 1px;
  }

  .preset-chip__label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: #c0c0d0;
    text-transform: uppercase;
  }

  .preset-chip--active .preset-chip__label {
    color: #3dffa0;
  }

  .preset-chip__dim {
    font-size: 9px;
    color: #44445a;
    letter-spacing: 0.04em;
  }

  .preset-chip--active .preset-chip__dim {
    color: #2a8a60;
  }

  /* ── Custom resolution inputs ─────────────────────────────────────── */
  .custom-resolution-inputs {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .custom-res-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .custom-res-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #33334a;
  }

  .custom-res-input {
    width: 100%;
  }

  .custom-res-sep {
    font-size: 18px;
    font-weight: 300;
    color: #33334a;
    padding-bottom: 7px;
    flex-shrink: 0;
    line-height: 1;
  }

  /* ── Resolution locked notice ─────────────────────────────────────── */
  .resolution-unsupported-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0a0d14;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
  }

  .resolution-unsupported-notice__icon {
    font-size: 11px;
    color: #6a6a88;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-unsupported-notice__text {
    font-size: 10px;
    color: #6a6a88;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-locked-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0a0d14;
    border: 1px solid #1e2640;
    border-radius: 4px;
  }

  .resolution-locked-notice__icon {
    font-size: 11px;
    color: #4a6aaa;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-locked-notice__text {
    font-size: 10px;
    color: #4a5a88;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-locked-notice__text strong {
    color: #6a8acc;
    font-weight: 700;
  }

  /* ── Resolution pending notice ────────────────────────────────────── */
  .resolution-pending-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0d0d0a;
    border: 1px solid #2a2a18;
    border-radius: 4px;
  }

  .resolution-pending-notice__icon {
    font-size: 11px;
    color: #7a7a40;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-pending-notice__text {
    font-size: 10px;
    color: #5a5a30;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Resolution warn notice (support lookup failed) ───────────────────── */
  .resolution-warn-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0d0b08;
    border: 1px solid #3a2a10;
    border-radius: 4px;
  }

  .resolution-warn-notice__icon {
    font-size: 11px;
    color: #c07820;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-warn-notice__text {
    font-size: 10px;
    color: #8a5820;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-supported-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0a1410;
    border: 1px solid #1a3020;
    border-radius: 4px;
  }

  .resolution-supported-notice__icon {
    font-size: 11px;
    color: #3dffa0;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-supported-notice__text {
    font-size: 10px;
    color: #2a8a60;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Video Bitrate chips ──────────────────────────────────────────── */
  .bitrate-mode-chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .bitrate-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 16px;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    font-family: inherit;
    transition: background 0.12s, border-color 0.12s;
    min-width: 72px;
  }

  .bitrate-chip:hover {
    background: #131320;
    border-color: #3a3a5a;
  }

  .bitrate-chip--active {
    background: #0d1f15;
    border-color: #3dffa0;
  }

  .bitrate-chip:focus-visible {
    outline: 1px solid #3dffa0;
    outline-offset: 1px;
  }

  .bitrate-chip__label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: #c0c0d0;
    text-transform: uppercase;
  }

  .bitrate-chip--active .bitrate-chip__label {
    color: #3dffa0;
  }

  .bitrate-chip__mbps {
    font-size: 9px;
    color: #44445a;
    letter-spacing: 0.04em;
  }

  .bitrate-chip--active .bitrate-chip__mbps {
    color: #2a8a60;
  }

  /* ── Bitrate preset hint ──────────────────────────────────────────── */
  .bitrate-preset-hint strong {
    color: #8080a0;
    font-weight: 700;
  }

  /* ── Custom bitrate input row ─────────────────────────────────────── */
  .custom-bitrate-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .custom-bitrate-input-wrap {
    display: flex;
    align-items: center;
    gap: 0;
  }

  .custom-bitrate-input {
    width: 120px;
    border-radius: 4px 0 0 4px;
    flex: unset;
  }

  .custom-bitrate-unit {
    padding: 7px 10px;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-left: none;
    border-radius: 0 4px 4px 0;
    font-size: 11px;
    color: #44445a;
    letter-spacing: 0.06em;
    white-space: nowrap;
    font-weight: 600;
    text-transform: uppercase;
    user-select: none;
  }

  /* ── Bitrate compatibility notice ─────────────────────────────────── */
  .bitrate-compat-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0a0d14;
    border: 1px solid #1e2640;
    border-radius: 4px;
  }

  .bitrate-compat-notice__icon {
    font-size: 11px;
    color: #4a6aaa;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .bitrate-compat-notice__text {
    font-size: 10px;
    color: #4a5a88;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Inactivity pause ─────────────────────────────────────────────── */
  .idle-timeout-row {
    margin-top: 2px;
  }

  /* ── Audio activity notice ────────────────────────────────────────── */
  .audio-activity-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: #0a100d;
    border: 1px solid #1a3028;
    border-radius: 4px;
  }

  .audio-activity-notice__icon {
    font-size: 11px;
    color: #3dffa0;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .audio-activity-notice__text {
    font-size: 10px;
    color: #2a7a58;
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .audio-activity-notice__text strong {
    color: #3dffa0;
    font-weight: 700;
  }

  /* ── Debug log status ────────────────────────────────────────────── */
  .debug-log-status {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: #0e0e16;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
  }

  .debug-log-status__row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .debug-log-status__label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #33334a;
    width: 48px;
    flex-shrink: 0;
  }

  .debug-log-status__value {
    font-size: 11px;
    color: #9090b0;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .debug-log-status__path {
    font-size: 10px;
    color: #6060a0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: monospace;
  }

  .debug-log-status__dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #33334a;
    flex-shrink: 0;
  }

  .debug-log-status__dot--on {
    background: #3dffa0;
  }

  .debug-log-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  /* ── Danger button variant ───────────────────────────────────────── */
  .btn--danger {
    background: #1e0a0a;
    color: #ff6b7a;
    border-color: #3a1a20;
  }

  .btn--danger:not(:disabled):hover {
    background: #2a1010;
    border-color: #ff6b7a;
  }
</style>
