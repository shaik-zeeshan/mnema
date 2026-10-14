// Every word the status popover ("What Mnema is doing") says, as pure functions
// over the state the status bar already reads. StatusPop.svelte only renders
// these. The Developer window keeps its own developer-voice copy; the undrawn
// capture rows below reuse its wording (`debug_health/rules.rs`) so both agree.

import type {
  CaptureHealthDto,
  CaptureSourceHealth,
  ProcessorPipelineStatus,
  SystemAudioTapRebuildReason,
} from "$lib/types/debug";
import type { QuickRecallIndexStatus } from "$lib/types/app-infra";
import { formatWait, gb } from "$lib/debug/capture-view";
import { isTransientError } from "$lib/debug/detail/jobs";

/** `{ num }` renders as `<span class="num">`. */
export type Part = string | { num: string };

export interface PopRow {
  name: string;
  /** ring = neutral, in progress, not a problem. */
  dot: "fill" | "ring";
  tone: "ok" | "warn" | "danger" | null;
  value: Part[];
  /** The `<small>` second line. */
  note?: Part[];
  action?: { label: string; settings: "storage" | "intelligence" | "transcription" };
}

export type CaptureGroup = { kind: "rows"; rows: PopRow[] } | { kind: "stopped" } | { kind: "starting" };
/** `null` = neither backlog could be read: the group is hidden. */
export type CatchingUp =
  | { kind: "caught-up" }
  | { kind: "busy"; rows: PopRow[]; meaningPercent: number | null }
  | null;

export interface PopInput {
  running: boolean;
  loadingStart: boolean;
  lowDisk: boolean;
  health: CaptureHealthDto | null;
  /** SP1's `sysAudioReconnecting`: false for a silence-watchdog rebuild whose tap is up. */
  sysAudioReconnecting: boolean;
  diskSoon: boolean;
  index: QuickRecallIndexStatus | null;
  pipeline: ProcessorPipelineStatus[] | null;
  /** `recordingSettings.transcription.provider` while transcription is enabled. */
  transcriptionProvider: string | null;
  deepgramAuthError: string | null;
  /** `engineFlag()` from status-bar.ts. */
  engine: { label: string; error: boolean; tip: string } | null;
}

const REBUILD: Record<SystemAudioTapRebuildReason, string> = {
  deviceChanged: "the output device changed",
  deviceDied: "the output device went away",
  sampleRateChanged: "the headset changed mode",
  excludeListMoved: "an excluded app started or quit",
  silenceWatchdog: "a long silence",
};

const plural = (n: number, one: string) => (n === 1 ? one : `${one}s`);
const ring = (name: string, value: Part[], note?: Part[]): PopRow => ({
  name,
  dot: "ring",
  tone: null,
  value,
  ...(note ? { note } : {}),
});
const fill = (name: string, tone: PopRow["tone"], value: Part[]): PopRow => ({ name, dot: "fill", tone, value });

/** A reconnecting tap that is actually up (founder ruling 1) still records. */
function recording(src: CaptureSourceHealth, sysAudio: boolean, i: PopInput): boolean {
  return src.state === "recording" || (sysAudio && src.state === "reconnecting" && !i.sysAudioReconnecting);
}

function sourceRow(name: string, src: CaptureSourceHealth, i: PopInput, sysAudio = false): PopRow {
  const h = i.health as CaptureHealthDto;
  if (recording(src, sysAudio, i)) return fill(name, "ok", ["Recording"]);
  switch (src.state) {
    case "reconnecting": {
      const why = h.systemAudioTap?.lastReason;
      return ring(name, [why ? `Reconnecting after ${REBUILD[why]}` : "Reconnecting"]);
    }
    case "paused":
      if (src.reason === "capture_low_disk" || i.lowDisk) return fill(name, "warn", ["Paused until you free up space"]);
      return ring(name, [src.reason === "user_paused" ? "Paused by you" : "Paused while you’re away"]);
    case "waiting": {
      if (src.reason === "privacy_filter_apply_failed") {
        return fill(name, "warn", ["Privacy filter couldn’t be applied, retrying"]);
      }
      const mic = recording(h.microphone, false, i);
      const sys = recording(h.systemAudio, true, i);
      const rest = mic && sys ? " Mic and system audio continue." : mic ? " Mic continues." : sys ? " System audio continues." : "";
      return ring(name, [`Waiting for the display.${rest}`]);
    }
    case "failed":
      return src.reason === "privacy_recovery_restart_required"
        ? fill(name, "danger", ["Stopped. Restart recording to resume"])
        : fill(name, "warn", ["Not capturing right now"]);
    default:
      return ring(name, ["Off for this recording"]);
  }
}

export function captureGroup(i: PopInput): CaptureGroup {
  if (!i.running) return i.loadingStart ? { kind: "starting" } : { kind: "stopped" };
  // ponytail: health unread (first poll pending or the read failed) hides the rows.
  if (!i.health) return { kind: "rows", rows: [] };
  return {
    kind: "rows",
    rows: [
      sourceRow("Screen", i.health.screen, i),
      sourceRow("Microphone", i.health.microphone, i),
      sourceRow("System audio", i.health.systemAudio, i, true),
    ],
  };
}

const lane = (i: PopInput, processor: string) => i.pipeline?.find((p) => p.processor === processor) ?? null;

/** "under a minute" · "about 2 min" · "about 2h 14m". */
function eta(seconds: number): Part[] {
  return seconds < 60 ? ["under a minute"] : ["about ", { num: formatWait(seconds * 1000) }];
}

export function catchingUp(i: PopInput): CatchingUp {
  if (!i.index && !i.pipeline) return null;
  const rows: PopRow[] = [];

  const frames = i.index?.queuedFrames ?? 0;
  if (frames > 0) {
    // ponytail: one job = one frame, serial throughput; ignores worker parallelism.
    const avg = lane(i, "ocr")?.averageCompletedSecondsLast24h ?? null;
    const value: Part[] = [{ num: String(frames) }, ` ${plural(frames, "frame")}`];
    if (avg != null) value.push(" · ", ...eta(frames * avg));
    rows.push(ring("Reading", value));
  }

  const audio = lane(i, "audio_transcription");
  const clips = audio ? audio.queued + audio.running : 0; // queued already includes retrying
  if (audio && clips > 0) {
    const n = { num: String(clips) };
    const word = ` ${plural(clips, "clip")}`;
    const deepgramDown = !!i.deepgramAuthError || isTransientError(audio.lastError);
    if (audio.retrying > 0 && i.transcriptionProvider === "deepgram" && deepgramDown) {
      rows.push({
        ...fill("Transcribing", "warn", [n, `${word} waiting`]),
        ...(i.deepgramAuthError
          ? {
              note: ["Deepgram rejected your key."],
              action: { label: "Transcription settings", settings: "transcription" as const },
            }
          : { note: ["Can’t reach Deepgram. Retries on its own."] }),
      });
    } else {
      rows.push(ring("Transcribing", [n, word], audio.retrying > 0 ? ["Retrying on its own."] : undefined));
    }
  }

  const meaningPercent =
    i.index?.semanticBuilding && i.index.semanticIndexedPercent != null ? i.index.semanticIndexedPercent : null;
  return rows.length === 0 && meaningPercent === null ? { kind: "caught-up" } : { kind: "busy", rows, meaningPercent };
}

/** Free space below this always warns. Not the backend's pause point (~1 GiB reserve + the next segment); only where we start warning. */
export const LOW_DISK_WARN_BYTES = 5 * 1024 ** 3;

export function diskRow(i: PopInput): PopRow | null {
  const disk = i.health?.disk;
  if (disk?.freeBytes == null) return null;
  const free = { num: `${gb(disk.freeBytes)} GB` };
  const storage = { label: "Open Storage", settings: "storage" as const };
  if (i.lowDisk) {
    return {
      ...fill("Free space", "warn", [free, " left"]),
      note: ["Recording is paused. It resumes on its own once you free up space."],
      action: storage,
    };
  }
  // Thresholds are null while stopped (e.g. after a disk-full stop), so a flat floor still warns.
  if ((i.diskSoon && disk.pauseThresholdBytes != null) || disk.freeBytes < LOW_DISK_WARN_BYTES) {
    return {
      ...fill("Free space", "warn", [free, " left"]),
      // Unknown threshold: 5 GB is only where we warn; the real pause is ~1 GB + the next segment.
      note:
        disk.pauseThresholdBytes == null
          ? ["Less than ", { num: `${gb(LOW_DISK_WARN_BYTES)} GB` }, " free. Recording pauses when the disk is nearly full."]
          : [
              "Recording pauses below ",
              { num: `${gb(disk.pauseThresholdBytes)} GB` },
              " and resumes when you free up space.",
            ],
      action: storage,
    };
  }
  return fill("Free space", null, [free]);
}

/** Only while the engine is unhealthy (same rule as the `mx-engine` pill); reuses its tip. */
export function aiRow(i: PopInput): PopRow | null {
  if (!i.engine) return null;
  const tone = i.engine.label === "needs reconnect" ? "danger" : i.engine.error ? "warn" : null;
  return {
    ...fill("Ask AI", tone, [i.engine.tip]),
    action: { label: "AI settings", settings: "intelligence" },
  };
}
