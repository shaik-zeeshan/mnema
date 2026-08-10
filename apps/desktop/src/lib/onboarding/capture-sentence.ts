// The arithmetic and copy behind onboarding's ONE capture line (issue #195,
// slice 6) — "Take a snapshot every 2 seconds, keep everything, in ~/.mnema."
//
// Design of record: `docs/onboarding/mockups/input-components/parts/sentence.part.html`.
// Behaviour and copy are ported from it; this module is the half that has no
// Svelte in it, so `bun test` can pin every number the sentence prints.
//
// The one rule that makes the component honest: the verdict below and the
// blocking predicate in `gates.ts` read the SAME `storageNeedBytes`, so the
// printed consequence and the reason Continue is held can never disagree.
import type { RetentionPolicy } from "../types";
import { formatBytes } from "../settings/state/format";
import { plural } from "./ai-readiness";
import { estimateDailyStorageMb } from "./disk-estimate";
import {
  RESERVE_FLOOR_BYTES,
  formatMeasuredBytes,
  storageNeedBytes,
  type StorageProbe,
} from "./gates";

/** Stops printed around the rate token, the chosen one included (mockup: 5). */
export const RATE_GHOST_WIDTH = 5;

/**
 * The label for "the folder the app picks when you pick nothing". The draft
 * save directory carries `""` for it and `probe_storage_path` resolves it, so
 * the literal path is a backend concern.
 *
 * ponytail: `~/.mnema` is `default_save_directory()`'s answer on every real
 * install; the `MNEMA_SAVE_DIRECTORY` dev knob would make this label lie, and
 * plumbing the resolved default through as a prop is not worth that case.
 */
export const DEFAULT_FOLDER_LABEL = "~/.mnema";

export interface RetentionStop {
  id: RetentionPolicy;
  /** How the sentence says it: "keep <word>,". */
  word: string;
  /** How a ghost says it, where there is no room for the sentence form. */
  short: string;
  days: number | null;
}

/**
 * `never` FIRST and default: retention is the only setting whose wrong guess
 * destroys data with no undo, so the app never defaults to deleting, and the
 * dial prints all four so "everything" reads as one of four rather than as a
 * statement. Deliberately a different order from `retentionPresets()`, which
 * is the Settings surface's ascending-duration list.
 */
export const RETENTION_STOPS: readonly RetentionStop[] = [
  { id: "never", word: "everything", short: "everything", days: null },
  { id: "days_30", word: "the last 30 days", short: "30 days", days: 30 },
  { id: "days_14", word: "the last 14 days", short: "14 days", days: 14 },
  { id: "days_7", word: "the last 7 days", short: "7 days", days: 7 },
];

export function retentionIndex(policy: RetentionPolicy): number {
  const i = RETENTION_STOPS.findIndex((stop) => stop.id === policy);
  return i < 0 ? 0 : i;
}

/** Daily capture cost in bytes — `disk-estimate`'s MB figure, not a re-derivation. */
export function dailyBytes(intervalSeconds: number, videoPixels?: number): number {
  return estimateDailyStorageMb(intervalSeconds, videoPixels) * 1e6;
}

/**
 * The slice of a ladder a token prints, `width` wide, centred on `index` and
 * clamped at both ends — so the ghost row never goes ragged at the ladder's
 * extremes and a ±2 jump is always one click away.
 */
export function ghostWindow(
  index: number,
  count: number,
  width: number,
): { start: number; end: number } {
  const w = Math.min(width, count);
  const start = Math.min(Math.max(index - ((w - 1) >> 1), 0), count - w);
  return { start, end: start + w };
}

/** "3 weeks" / "8 months" / "1.4 years" — how long a volume lasts at this rate. */
export function humanDuration(days: number): string {
  if (!Number.isFinite(days) || days <= 0) return "no time at all";
  if (days < 1) return "less than a day";
  if (days < 14) return plural(Math.round(days), "day");
  if (days < 60) return plural(Math.round(days / 7), "week");
  if (days < 365) return plural(Math.round(days / 30), "month");
  // `Number` drops the trailing `.0`, so a flat year reads "1 year", not "1.0 years".
  return plural(Number((days / 365).toFixed(1)), "year");
}

/**
 * The volume a path lives on, named the way a user names it. `probe_storage_path`
 * returns no volume name and macOS puts every non-boot mount under `/Volumes/`,
 * so the name is the mount segment.
 */
export function volumeLabel(path: string): string {
  return /^\/Volumes\/([^/]+)/.exec(path)?.[1] ?? "your startup disk";
}

/**
 * The path as a word inside a sentence: home-relative, and with the middle
 * dropped rather than the end, so the folder you actually chose stays readable.
 * The full path stays available as the token's `title`.
 */
export function sentencePath(path: string, max = 28): string {
  const home = /^(\/Users\/[^/]+)/.exec(path)?.[1];
  const shown = home ? `~${path.slice(home.length)}` : path;
  if (shown.length <= max) return shown;
  const parts = shown.split("/");
  if (parts.length <= 2) return `…${shown.slice(-(max - 1))}`;
  const head = parts[0] ?? "";
  const long = `${head}/…/${parts.slice(-2).join("/")}`;
  return long.length > max ? `${head}/…/${parts[parts.length - 1]}` : long;
}

/** One run of verdict text; `kind` is emphasis, not markup the caller parses. */
export interface Seg {
  text: string;
  kind?: "fig" | "verdict";
}

export type RepairAct =
  | "default"
  | "pick"
  | "recheck"
  | "nosemantic"
  | "slower"
  | "tighten"
  | "keep30";

export interface Repair {
  label: string;
  act: RepairAct;
  primary: boolean;
}

export type Tone = "" | "ok" | "warn" | "bad";
export type ProbeState = "checking" | "done" | "failed";

export interface SentenceVerdict {
  tone: Tone;
  /** The probe is in flight: the sentence trails off instead of ending. */
  probing: boolean;
  /** The clause that BREAKS the sentence instead of letting it reach a full
   *  stop, or `null` when nothing is wrong with the folder. */
  clause: string | null;
  /** Always `rate · horizon`, whatever retention is. */
  plan: Seg[];
  verdict: Seg[];
  repairs: Repair[];
  /**
   * Whether `captureStorageBlockReason` is holding Continue for this probe.
   * An inability to MEASURE is never a shortfall (ADR 0040), so a failed probe
   * and an unreadable free-space figure both report `false`.
   */
  blocking: boolean;
}

export interface SentenceInput {
  intervalSeconds: number;
  /** Pixels per captured frame (`draftVideoPixels`). Omitted = anchor 720p. */
  videoPixels?: number;
  retention: RetentionPolicy;
  /** The path the probe measured (or the draft, before the first reply). */
  path: string;
  probe: StorageProbe | null;
  probeState: ProbeState;
  /** `flow.downloadBytes` — what the downloads will fetch. */
  requiredBytes: number;
  /** Only its presence changes copy: a total containing nomic reads "about". */
  semanticSearchOn: boolean;
}

const t = (text: string): Seg => ({ text });
const fig = (text: string): Seg => ({ text, kind: "fig" });
const verd = (text: string): Seg => ({ text, kind: "verdict" });
const repair = (label: string, act: RepairAct, primary = false): Repair => ({
  label,
  act,
  primary,
});

/**
 * The plan line ALWAYS reads `rate · horizon`. With retention `never` the
 * horizon is a first-year projection, not a missing ceiling — so the default
 * case has a number that moves, in the same position, and retention's job reads
 * as "trade a growing projection for a fixed ceiling".
 */
export function planSegments(
  intervalSeconds: number,
  retention: RetentionPolicy,
  videoPixels?: number,
): Seg[] {
  const daily = dailyBytes(intervalSeconds, videoPixels);
  const stop = RETENTION_STOPS[retentionIndex(retention)]!;
  const head = [fig(formatBytes(daily)), t(" a day")];
  return stop.days === null
    ? [
        ...head,
        t(" · about "),
        fig(formatBytes(daily * 365)),
        t(" in the first year, and it keeps going"),
      ]
    : [
        ...head,
        t(" · "),
        fig(formatBytes(daily * stop.days)),
        t(" held, then it stops growing"),
      ];
}

/**
 * Everything the sentence says below itself, for one (setting, probe) pair.
 * Ordered exactly as `captureStorageBlockReason` tests its cases, so the panel
 * and the gate always name the same problem.
 */
export function sentenceVerdict(input: SentenceInput): SentenceVerdict {
  const daily = dailyBytes(input.intervalSeconds, input.videoPixels);
  const stop = RETENTION_STOPS[retentionIndex(input.retention)]!;
  const held = stop.days === null ? null : daily * stop.days;
  const plan = planSegments(input.intervalSeconds, input.retention, input.videoPixels);
  const base = { probing: false, clause: null, plan, blocking: false } as const;
  const probe = input.probe;

  if (input.probeState === "checking") {
    return { ...base, tone: "", probing: true, verdict: [], repairs: [] };
  }
  if (input.probeState === "failed" || !probe) {
    // Distinct from "not yet probed": the check RAN and broke. It still cannot
    // block (ADR 0040) — it just must not keep saying "checking…" forever.
    return {
      ...base,
      tone: "warn",
      verdict: [
        verd("Mnema couldn't check that folder."),
        t(
          " That doesn't block anything — the plan above still holds, it just can't be checked against the disk.",
        ),
      ],
      repairs: [repair("Re-check", "recheck", true)],
    };
  }

  const volume = volumeLabel(input.path);
  const onVolume = /^\/Volumes\/[^/]+/.test(input.path);

  // A disconnected volume is the one case where the folder is missing AND the
  // free space is unreadable: `measure_free_space` refuses to walk past
  // `/Volumes/<name>`, so it reports nothing rather than the boot disk's bytes.
  if (onVolume && !probe.exists && probe.freeBytes === null) {
    return {
      ...base,
      tone: "bad",
      clause: "that drive isn't connected right now.",
      blocking: true,
      verdict: [
        t(
          "Mnema won't record to a volume it can't see, and if it disappears mid-session recording stops until it's back.",
        ),
      ],
      repairs: [
        repair(`Use ${DEFAULT_FOLDER_LABEL} instead`, "default", true),
        repair("Re-check", "recheck"),
      ],
    };
  }
  if (!probe.exists) {
    return {
      ...base,
      tone: "bad",
      clause: "that folder isn't there yet.",
      blocking: true,
      verdict: [
        t(
          probe.freeBytes === null
            ? "Free space unknown"
            : `${formatMeasuredBytes(probe.freeBytes)} free`,
        ),
        t(` on ${volume} — only the folder is missing.`),
      ],
      repairs: [
        repair("Choose another folder…", "pick", true),
        repair("Re-check", "recheck"),
      ],
    };
  }
  if (!probe.writable) {
    return {
      ...base,
      tone: "bad",
      clause: `${volume} is read-only.`,
      blocking: true,
      verdict: [t("The write probe failed there — proven, not guessed.")],
      repairs: [
        repair("Choose another folder…", "pick", true),
        repair(`Use ${DEFAULT_FOLDER_LABEL} instead`, "default"),
      ],
    };
  }
  if (probe.freeBytes === null) {
    // NOT a break: an inability to measure is not a shortfall (ADR 0040).
    return {
      ...base,
      tone: "warn",
      verdict: [
        verd(`Free space on ${volume} couldn't be read.`),
        t(
          " That doesn't block anything — the plan above still holds, it just can't be checked against the disk.",
        ),
      ],
      repairs: [repair("Re-check", "recheck")],
    };
  }

  const free = probe.freeBytes;
  const downloads = input.requiredBytes;
  const need = storageNeedBytes(
    downloads,
    input.intervalSeconds,
    input.videoPixels,
  );
  const usable = free - downloads - RESERVE_FLOOR_BYTES;
  const about = input.semanticSearchOn ? "about " : "";

  if (free < need) {
    // Same split as `captureStorageBlockReason`: the models not fitting and
    // there being nowhere to record are different problems with different fixes.
    // An empty work-list has no download clause to print — `formatBytes(0)` is
    // the app's "can't determine" sentinel, so it would read "unknown size".
    if (downloads > 0 && free < RESERVE_FLOOR_BYTES + downloads) {
      return {
        ...base,
        tone: "bad",
        clause: "there isn't room for the models yet.",
        blocking: true,
        verdict: [
          verd(
            `${formatBytes(downloads + RESERVE_FLOOR_BYTES - free)} short before recording even starts.`,
          ),
          t(` The downloads are ${about}`),
          fig(formatBytes(downloads)),
          t(`; ${volume} has `),
          fig(formatMeasuredBytes(free)),
          t(" free."),
        ],
        repairs: [
          ...(input.semanticSearchOn
            ? [repair("Turn Semantic Search off", "nosemantic", true)]
            : []),
          repair("Choose another folder…", "pick", !input.semanticSearchOn),
        ],
      };
    }
    return {
      ...base,
      tone: "bad",
      clause: "there isn't room for a day of recording.",
      blocking: true,
      verdict: [
        verd("The downloads fit; the capture does not."),
        t(` After ${about}`),
        fig(formatBytes(downloads + RESERVE_FLOOR_BYTES)),
        t(" of models and safety reserve, "),
        ...(usable <= 0
          ? [t("nothing is left")]
          : [t("only "), fig(formatBytes(usable)), t(" is left")]),
        t(" — and one day costs "),
        fig(formatBytes(daily)),
        t("."),
      ],
      repairs: [
        repair("Choose another folder…", "pick", true),
        repair("Record less often", "slower"),
      ],
    };
  }

  if (held !== null && held > usable) {
    return {
      ...base,
      tone: "warn",
      verdict: [
        t(`Holding ${stop.short} takes about `),
        fig(formatBytes(held)),
        t(", and "),
        fig(formatBytes(usable)),
        t(` is free on ${volume}. `),
        verd(`It fills up after about ${humanDuration(usable / daily)}.`),
      ],
      repairs: [
        repair("Record less often", "slower", true),
        repair("Keep less", "tighten"),
      ],
    };
  }
  if (held === null) {
    const runout = usable / daily;
    return {
      ...base,
      tone: runout >= 180 ? "ok" : "warn",
      verdict: [
        t(`${formatBytes(free)} free on ${volume} — `),
        verd(`room for about ${humanDuration(runout)} at this rate`),
        t(", and nothing is deleted to make more."),
      ],
      repairs:
        runout >= 180
          ? []
          : [
              repair("Keep the last 30 days", "keep30", true),
              repair("Record less often", "slower"),
            ],
    };
  }
  return {
    ...base,
    tone: "ok",
    verdict: [
      t(`${formatBytes(free)} free on ${volume} — `),
      verd(`fits, with ${formatBytes(usable - held)} to spare`),
      t(` (and it stops growing after ${stop.days} days).`),
    ],
    repairs: [],
  };
}
