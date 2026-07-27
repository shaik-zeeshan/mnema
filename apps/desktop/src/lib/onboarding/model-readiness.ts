// Model readiness + aggregate download progress (issue #195, slice 3).
//
// The bug this whole issue exists for: `onboarding-attention.ts` classifies a
// DOWNLOADING model identically to a MISSING one (both are just `!available`),
// so a download in flight counts as an unresolved attention item and blocks the
// user from leaving the step. Here they are FOUR distinct states, and none of
// them blocks finishing — only `missing` and `failed` are even worth rendering
// on the Setup screen.
//
// Plus one pure reducer folding the four per-subsystem progress streams into a
// single byte-weighted percent and a current-item label. Progress never moves
// backwards, events may arrive out of order, and streams run concurrently.
import type { DownloadWorkItem } from "./resolve-setup";

/** Mirrors the identical `*ModelDownloadStatus` unions in `$lib/types` — all
 *  four subsystems emit the same six values. */
export type DownloadStatus =
  | "starting"
  | "downloading"
  | "installing"
  | "completed"
  | "failed"
  | "cancelled";

/** The four states. `downloading` is NOT `missing`. */
export type ReadinessState = "ready" | "downloading" | "missing" | "failed";

/**
 * Classify one model. `installed` is the backend's `available` flag; `download`
 * is the live progress event for that model, if any is in flight.
 *
 * Nothing here gates finishing — the Setup screen's Continue is live on arrival
 * and never disables. See `isSetupNoteworthy` for what is worth showing.
 */
export function classifyReadiness(
  installed: boolean,
  download: { status: DownloadStatus } | null | undefined,
): ReadinessState {
  if (installed) return "ready";
  switch (download?.status) {
    case "starting":
    case "downloading":
    case "installing":
      return "downloading";
    // `completed` before the status reload lands: the bytes are on disk.
    case "completed":
      return "ready";
    case "failed":
      return "failed";
    // `cancelled`, or no download at all.
    default:
      return "missing";
  }
}

/** Only `missing` and `failed` are interesting on the Setup screen. A download
 *  in flight is progress, not a problem, and nothing blocks Continue. */
export function isSetupNoteworthy(state: ReadinessState): boolean {
  return state === "missing" || state === "failed";
}

// ── Aggregate progress ─────────────────────────────────────────────────────

/** One event from any of the four per-subsystem progress streams. All four
 *  `*ModelDownloadProgress` types are structurally this. */
export interface DownloadProgressEvent {
  provider: string;
  modelId: string;
  status: DownloadStatus;
  downloadedBytes: number;
  totalBytes: number | null;
  message: string | null;
}

/** Byte-weighted aggregate over the whole work-list. Treat as immutable. */
export interface DownloadProgressState {
  readonly items: readonly DownloadWorkItem[];
  /** 0-100, byte-weighted, MONOTONIC — never moves backwards. */
  readonly percent: number;
  /** Bytes received per work-item id, clamped to the item total and monotonic. */
  readonly received: Readonly<Record<string, number>>;
  readonly states: Readonly<Record<string, ReadinessState>>;
  /** Real error text per failed work-item id — surfaced AT the failed item. */
  readonly errors: Readonly<Record<string, string>>;
  /** Label of the item currently downloading, else the next unfinished one. */
  readonly currentLabel: string | null;
  /** Every item is `ready`. An empty work-list is done immediately. */
  readonly done: boolean;
}

export function startProgress(
  items: readonly DownloadWorkItem[],
): DownloadProgressState {
  const received: Record<string, number> = {};
  const states: Record<string, ReadinessState> = {};
  for (const item of items) {
    received[item.id] = 0;
    states[item.id] = "missing";
  }
  return finish({ items, received, states, errors: {}, percent: 0 });
}

/**
 * Fold one event in. Events for models not on the work-list are ignored (the
 * user may start an unrelated download from Settings mid-flow).
 */
export function applyProgressEvent(
  state: DownloadProgressState,
  event: DownloadProgressEvent,
): DownloadProgressState {
  const item = state.items.find(
    (candidate) =>
      candidate.provider === event.provider && candidate.modelId === event.modelId,
  );
  if (!item) return state;

  const received = { ...state.received };
  const states = { ...state.states };
  const errors = { ...state.errors };

  const previousBytes = received[item.id] ?? 0;
  const eventBytes =
    event.status === "completed"
      ? item.bytes
      : Math.min(Math.max(event.downloadedBytes, 0), item.bytes);
  // Monotonic per item: an out-of-order "40%" arriving after "80%" is dropped.
  received[item.id] = Math.max(previousBytes, eventBytes);

  // `ready` is terminal — a late event from a finished stream can't un-finish it.
  if (states[item.id] !== "ready") {
    states[item.id] = classifyReadiness(false, event);
  }
  if (event.status === "failed") {
    errors[item.id] = event.message ?? "Download failed.";
  } else if (event.status === "starting" || event.status === "downloading") {
    // A retry clears the previous attempt's error.
    delete errors[item.id];
  }

  return finish({
    items: state.items,
    received,
    states,
    errors,
    percent: state.percent,
  });
}

function finish(input: {
  items: readonly DownloadWorkItem[];
  received: Record<string, number>;
  states: Record<string, ReadinessState>;
  errors: Record<string, string>;
  percent: number;
}): DownloadProgressState {
  const { items, received, states, errors } = input;
  const totalBytes = items.reduce((sum, item) => sum + item.bytes, 0);
  const receivedBytes = items.reduce(
    (sum, item) => sum + (received[item.id] ?? 0),
    0,
  );
  const rawPercent = totalBytes > 0 ? (receivedBytes / totalBytes) * 100 : 100;
  // Monotonic in aggregate too, independent of the per-item clamp.
  const percent = Math.max(input.percent, Math.round(rawPercent * 10) / 10);

  const downloading = items.find((item) => states[item.id] === "downloading");
  const pending = items.find((item) => states[item.id] !== "ready");
  const current = downloading ?? pending ?? null;

  return {
    items,
    received,
    states,
    errors,
    percent,
    currentLabel: current ? current.label : null,
    done: items.every((item) => states[item.id] === "ready"),
  };
}
