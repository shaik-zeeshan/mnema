// Onboarding download-progress / settings-changed event wiring. Lifted 1:1 out
// of `OnboardingController.startListeners` to keep that file under the size
// budget; the controller method now delegates here. Behavior is identical: it
// subscribes to the four model-download-progress events plus the recording
// settings-changed event and returns a single combined unlisten for the
// +page's `$effect` cleanup, guarding against an async resolve landing after the
// effect/component is torn down.
import { listen } from "@tauri-apps/api/event";
import type {
  AudioTranscriptionModelDownloadProgress,
  OcrModelDownloadProgress,
  RecordingSettings,
  SemanticSearchModelDownloadProgress,
  SpeakerAnalysisModelDownloadProgress,
} from "$lib/types";
import {
  AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
  OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
  RECORDING_SETTINGS_CHANGED_EVENT,
  SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
  SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
} from "./onboarding-mapping";

// The exact slice of the controller the listeners drive: the per-model progress
// handlers plus a settings sink for the recording-settings-changed event.
export interface OnboardingListenerTarget {
  handleOcrDownloadProgress(payload: OcrModelDownloadProgress): void;
  handleTranscriptionDownloadProgress(payload: AudioTranscriptionModelDownloadProgress): void;
  handleSpeakerDownloadProgress(payload: SpeakerAnalysisModelDownloadProgress): void;
  handleSemanticSearchDownloadProgress(payload: SemanticSearchModelDownloadProgress): void;
  settings: RecordingSettings | null;
  // Optional Gecko browser-URL access: re-polled on window focus (the grant is
  // completed outside the app in System Settings).
  readonly geckoTrusted: boolean;
  recheckGeckoAccess(): Promise<void>;
}

export async function startOnboardingListeners(
  target: OnboardingListenerTarget,
): Promise<() => void> {
  let unlistenOcrDownloadProgress: (() => void) | undefined;
  let unlistenTranscriptionDownloadProgress: (() => void) | undefined;
  let unlistenSpeakerDownloadProgress: (() => void) | undefined;
  let unlistenSemanticSearchDownloadProgress: (() => void) | undefined;
  let unlistenRecordingSettingsChanged: (() => void) | undefined;
  let destroyed = false;

  // Accessibility is granted outside the app (System Settings), so re-poll on
  // window focus to pick up a grant without making the user click Recheck. Skip
  // once trusted; the controller's in-flight latch keeps refocus storms from
  // double-firing.
  const onWindowFocus = () => {
    if (!target.geckoTrusted) void target.recheckGeckoAccess();
  };
  const hasWindow = typeof window !== "undefined";
  if (hasWindow) window.addEventListener("focus", onWindowFocus);

  const unlisten = () => {
    destroyed = true;
    unlistenOcrDownloadProgress?.();
    unlistenTranscriptionDownloadProgress?.();
    unlistenSpeakerDownloadProgress?.();
    unlistenSemanticSearchDownloadProgress?.();
    unlistenRecordingSettingsChanged?.();
    if (hasWindow) window.removeEventListener("focus", onWindowFocus);
  };

  await Promise.all([
    listen<OcrModelDownloadProgress>(OCR_MODEL_DOWNLOAD_PROGRESS_EVENT, (event) => {
      void target.handleOcrDownloadProgress(event.payload);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenOcrDownloadProgress = fn;
    }),
    listen<AudioTranscriptionModelDownloadProgress>(
      AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => { void target.handleTranscriptionDownloadProgress(event.payload); },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenTranscriptionDownloadProgress = fn;
    }),
    listen<SpeakerAnalysisModelDownloadProgress>(
      SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => { void target.handleSpeakerDownloadProgress(event.payload); },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSpeakerDownloadProgress = fn;
    }),
    listen<SemanticSearchModelDownloadProgress>(
      SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => { void target.handleSemanticSearchDownloadProgress(event.payload); },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSemanticSearchDownloadProgress = fn;
    }),
    listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      target.settings = event.payload;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    }),
  ]);

  return unlisten;
}
