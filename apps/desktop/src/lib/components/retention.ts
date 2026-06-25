import type { RetentionPolicy } from "$lib/types";

/**
 * The retention duration presets, in display order.
 *
 * This is a restyle of the EXISTING closed `RetentionPolicy` enum
 * (`"never" | "days_7" | "days_14" | "days_30"`) — not a new feature. The four
 * entries here are exactly the persistable backend values, surfaced as friendly
 * duration labels. The bounded `never` reads as "Forever".
 *
 * Order: ascending duration, with the unbounded "Forever" last.
 */
export interface RetentionPreset {
  value: RetentionPolicy;
  /** Friendly label shown in the picker (e.g. "7 days", "Forever"). */
  label: string;
}

export function retentionPresets(): RetentionPreset[] {
  return [
    { value: "days_7", label: "7 days" },
    { value: "days_14", label: "14 days" },
    { value: "days_30", label: "30 days" },
    { value: "never", label: "Forever" },
  ];
}

/**
 * The retention window in days for a policy, or `null` for an unbounded
 * ("Forever") policy. Pure helper kept here (a plain `.ts`) so it is unit
 * testable — `bun test` cannot import from `.svelte` files.
 */
export function retentionToDays(policy: RetentionPolicy): number | null {
  switch (policy) {
    case "days_7":
      return 7;
    case "days_14":
      return 14;
    case "days_30":
      return 30;
    case "never":
      return null;
  }
}

/** The friendly label for a policy (e.g. for a trigger or summary line). */
export function retentionLabel(policy: RetentionPolicy): string {
  return retentionPresets().find((p) => p.value === policy)?.label ?? "Forever";
}
