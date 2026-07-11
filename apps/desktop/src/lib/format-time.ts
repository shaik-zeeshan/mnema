/** Parse a capture timestamp that may arrive as either a SQLite-style
 *  "YYYY-MM-DD HH:MM:SS" string or an ISO "YYYY-MM-DDTHH:MM:SS" string into a
 *  `Date`. The space form is normalized to the `T` form so `new Date` parses it
 *  consistently across engines. An unparseable input yields an invalid `Date`
 *  (callers gate on `isNaN(d.getTime())`). */
export function parseCapturedAt(ts: string): Date {
  return new Date(ts.includes("T") ? ts : ts.replace(" ", "T"));
}

/** Coarse relative age ("12m ago", "4h ago", "3d ago") used by the search
 *  result rows' accessory column, following the same buckets as the insights
 *  surfaces. Sub-minute (and any future timestamp from clock skew) reads
 *  "just now"; an unparseable input falls back to the raw string. `now` is
 *  injectable for tests. */
export function formatRelativeTime(ts: string, now: Date = new Date()): string {
  const d = parseCapturedAt(ts);
  if (isNaN(d.getTime())) return ts;
  const min = Math.floor((now.getTime() - d.getTime()) / 60_000);
  if (min < 1) return "just now";
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  const wk = Math.floor(day / 7);
  if (wk < 5) return `${wk}w ago`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.floor(day / 365)}y ago`;
}

/** Localized compact timestamp ("Jun 3, 2:05 PM") used by the answer-source and
 *  search-result cards and the dashboard's compact readout. Falls back to the
 *  raw string when the timestamp does not parse, so a displayed value is never
 *  silently dropped. */
export function formatTimestampCompact(ts: string): string {
  const d = parseCapturedAt(ts);
  if (isNaN(d.getTime())) return ts;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
