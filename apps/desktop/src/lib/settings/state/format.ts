// Pure formatting/label helpers shared across the Settings state modules.
// No Svelte runes, no `invoke` — safe to unit-test directly.

// Normalize an unknown thrown value into a human-facing message.
export function describeError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string")
    return err.message;
  if (err instanceof Error && err.message) return err.message;
  return "Something went wrong. Please try again.";
}

// JSON-or-string form of a thrown value. Used by loaders that surface the raw
// error verbatim (model/log status panels) rather than the friendly form above.
export function errorText(err: unknown): string {
  return typeof err === "string" ? err : JSON.stringify(err, null, 2);
}

// Human-readable byte size (B/KB/MB/GB). Returns "unknown size" for
// non-finite/non-positive inputs so callers don't render "NaN".
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "unknown size";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
