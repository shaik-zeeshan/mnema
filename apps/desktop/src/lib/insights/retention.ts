// Footage-retention labels (OV-14, JR-22), shared by Overview and the Journal.
import type { RetentionPolicy } from "$lib/types/recording";

const DAY_MS = 86_400_000;
const POLICY_DAYS: Record<RetentionPolicy, number | null> = {
  never: null,
  days_7: 7,
  days_14: 14,
  days_30: 30,
};

export interface RetentionVerdict {
  /** "removed": the span ends before the cutoff; "partly": it straddles it. */
  kind: "removed" | "partly";
  days: number;
}

/**
 * Whether missing footage in `[startMs, endMs)` is explained by retention.
 * `null` → not retention's doing: say "no screen capture" / "audio only".
 *
 * ponytail: a guess from today's policy — a policy change, or a cleanup that
 * hasn't run yet, can mislabel a span. Accepted; the upgrade is a per-segment
 * "deleted by retention" record.
 */
export function retentionVerdict(
  startMs: number,
  endMs: number,
  policy: RetentionPolicy | null | undefined,
  nowMs: number = Date.now(),
): RetentionVerdict | null {
  const days = policy ? POLICY_DAYS[policy] : null;
  if (days == null) return null;
  const cutoffMs = nowMs - days * DAY_MS;
  if (endMs <= cutoffMs) return { kind: "removed", days };
  if (startMs < cutoffMs) return { kind: "partly", days };
  return null;
}
