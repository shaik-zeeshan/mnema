/** Parse a capture timestamp that may arrive as either a SQLite-style
 *  "YYYY-MM-DD HH:MM:SS" string or an ISO "YYYY-MM-DDTHH:MM:SS" string into a
 *  `Date`. The space form is normalized to the `T` form so `new Date` parses it
 *  consistently across engines. An unparseable input yields an invalid `Date`
 *  (callers gate on `isNaN(d.getTime())`). */
export function parseCapturedAt(ts: string): Date {
  return new Date(ts.includes("T") ? ts : ts.replace(" ", "T"));
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
