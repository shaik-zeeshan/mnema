// Turn ANY thrown / rejected value into a single, human-facing message line.
//
// Tauri `invoke` rejections are often not `Error` instances — they can be a
// plain string, a serde-serialized Rust error struct/enum, or a JSON-encoded
// string. The old code rendered those with `JSON.stringify(err)`, leaking raw
// JSON (or `[object Object]`) into the UI. This module is the single place that
// knows how to dig a readable sentence out of those shapes.
//
// Pure (no Svelte runes, no `invoke`) — safe to import anywhere and unit-test.

export const GENERIC_ERROR_MESSAGE = "Something went wrong. Please try again.";

const MAX_LENGTH = 300;
const MAX_DEPTH = 6;

// Keys Rust/JS errors commonly carry the human message under, in priority order.
const MESSAGE_KEYS = [
  "message",
  "error",
  "reason",
  "detail",
  "details",
  "description",
  "msg",
  "cause",
] as const;

// Normalize an unknown thrown value into a friendly, single-line message.
// Pass a custom `fallback` for surfaces that want something more specific than
// the generic default when nothing readable can be extracted.
export function humanizeError(err: unknown, fallback: string = GENERIC_ERROR_MESSAGE): string {
  return tidy(extract(err, 0)) || fallback;
}

function extract(err: unknown, depth: number): string {
  if (depth > MAX_DEPTH || err === null || err === undefined) return "";
  if (typeof err === "string") return fromString(err, depth);
  if (typeof err === "number" || typeof err === "boolean") return String(err);
  if (err instanceof Error) return fromString(err.message, depth) || err.name || "";
  if (Array.isArray(err)) {
    for (const item of err) {
      const found = extract(item, depth + 1);
      if (found) return found;
    }
    return "";
  }
  if (typeof err === "object") return fromObject(err as Record<string, unknown>, depth);
  return "";
}

function fromString(value: string, depth: number): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  // A JSON-encoded error (e.g. a Rust error serialized to a string): parse it
  // and recurse so we surface the inner message instead of the JSON text.
  const looksJson =
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"));
  if (looksJson) {
    try {
      const inner = extract(JSON.parse(trimmed), depth + 1);
      if (inner) return inner;
    } catch {
      // Not actually JSON — fall through and use the raw string.
    }
  }
  return trimmed;
}

function fromObject(obj: Record<string, unknown>, depth: number): string {
  // Common shapes: { message }, { error }, { reason }, { detail }, ...
  for (const key of MESSAGE_KEYS) {
    if (!(key in obj)) continue;
    const value = obj[key];
    if (typeof value === "string" && value.trim()) return fromString(value, depth);
    if (value && typeof value === "object") {
      const nested = extract(value, depth + 1);
      if (nested) return nested;
    }
  }
  // Serde externally-tagged enum: a single key whose name IS the variant
  // (e.g. { PermissionDenied: "..." } or { Io: { ... } }).
  const keys = Object.keys(obj);
  if (keys.length === 1) {
    const label = humanizeIdentifier(keys[0]);
    const payload = extract(obj[keys[0]], depth + 1);
    if (payload && payload.toLowerCase() !== label.toLowerCase()) return `${label}: ${payload}`;
    return payload || label;
  }
  return "";
}

// "PermissionDenied" / "permission_denied" / "permission-denied" → "Permission denied"
function humanizeIdentifier(key: string): string {
  const words = key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim()
    .toLowerCase();
  if (!words) return key;
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function tidy(message: string): string {
  let m = message.replace(/\s+/g, " ").trim();
  if (!m) return "";
  // Drop a redundant leading "Error:" / "error -" prefix.
  m = m.replace(/^error\s*[:\-]\s*/i, "").trim();
  if (!m) return "";
  m = m.charAt(0).toUpperCase() + m.slice(1);
  if (m.length > MAX_LENGTH) m = `${m.slice(0, MAX_LENGTH - 1).trimEnd()}…`;
  return m;
}
