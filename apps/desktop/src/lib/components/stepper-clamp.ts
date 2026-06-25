// Pure clamp/parse logic for the Stepper number control.
//
// The Stepper feeds RAW STRINGS upward (the settings shell parses those strings
// with its own integer regex), so these helpers stay string-in / string-out and
// preserve the "blank = unset" contract: an empty (or whitespace-only) raw value
// maps to the empty string, never to a clamped number.

export interface ClampOptions {
  min?: number;
  max?: number;
}

/** Parse a raw stepper string to an integer, or null when blank/non-numeric. */
export function parseStepperRaw(raw: string): number | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  if (!/^-?\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return Number.isInteger(value) ? value : null;
}

/**
 * Clamp a raw string value to [min, max].
 *
 * - Blank / whitespace-only -> "" (preserves "blank = unset").
 * - Non-numeric -> returned unchanged (let the upstream validator surface the
 *   error; do not silently rewrite what the user typed into a number).
 * - Valid integer -> clamped into range and re-serialised as a string.
 */
export function clampToRange(raw: string, { min, max }: ClampOptions = {}): string {
  if (raw.trim() === "") return "";
  const parsed = parseStepperRaw(raw);
  if (parsed === null) return raw;
  return String(clampNumber(parsed, min, max));
}

/** Clamp a number into [min, max] (either bound optional). */
export function clampNumber(value: number, min?: number, max?: number): number {
  let next = value;
  if (typeof min === "number" && next < min) next = min;
  if (typeof max === "number" && next > max) next = max;
  return next;
}

/**
 * Compute the next raw value when a +/- button is pressed.
 *
 * Steps from the current parsed value, or seeds from `min` (then 0) when the
 * field is blank/invalid so the first click always produces an in-range number.
 */
export function stepRaw(
  raw: string,
  direction: 1 | -1,
  step: number,
  { min, max }: ClampOptions = {},
): string {
  const current = parseStepperRaw(raw);
  const base = current ?? (typeof min === "number" ? min : 0);
  // When seeding from a blank field, the seed itself is the first value rather
  // than seed +/- step, so the first click lands on min (or 0) cleanly.
  const next = current === null ? base : base + direction * step;
  return String(clampNumber(next, min, max));
}
