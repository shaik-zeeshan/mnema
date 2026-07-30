// Pure formatting/label helpers shared across the Settings state modules.
// No Svelte runes, no `invoke` — safe to unit-test directly.

import { humanizeError } from "$lib/format-error";

// Normalize an unknown thrown value into a human-facing message.
// Both names delegate to the shared humanizer so error surfaces never render
// raw JSON / `[object Object]`. `errorText` is kept for its existing callers.
export function describeError(err: unknown): string {
  return humanizeError(err);
}

export function errorText(err: unknown): string {
  return humanizeError(err);
}

// Human-readable byte size — the app's ONE byte formatter (onboarding used to
// carry a second, decimal one). Decimal/SI, because every figure it renders is
// SI at the source: the model manifests quote decimal bytes (Whisper base
// 147,951,465 = 148 MB) and macOS reports volume free space the same way.
// Runs to PB so a multi-terabyte volume reads "1.4 TB", not "1322.5 GB".
// Returns "unknown size" for non-finite/non-positive inputs so callers don't
// render "NaN".
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "unknown size";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let size = value;
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
