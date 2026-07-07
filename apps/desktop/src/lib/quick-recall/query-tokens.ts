// Pure query-token helpers for Quick Recall's search mode (extracted from the
// quick-recall +page.svelte in the search-mode extraction slice — no behavior
// change). Everything here is a pure function of its inputs: tokenizing the raw
// query, inspecting the trailing token, and formatting operator values/dates.

// Split `raw` into whitespace-delimited tokens, but keep a double-quoted run
// as ONE token so a quoted operator value containing spaces (e.g.
// `app:"Google Chrome"`) is never shredded mid-value. An unterminated quote
// still flushes its trailing run rather than dropping it.
export function tokenizeQuery(raw: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let inQuotes = false;
  for (const ch of raw) {
    if (ch === '"') {
      inQuotes = !inQuotes;
      current += ch;
    } else if (!inQuotes && /\s/.test(ch)) {
      if (current.length > 0) {
        tokens.push(current);
        current = "";
      }
    } else {
      current += ch;
    }
  }
  if (current.length > 0) {
    tokens.push(current);
  }
  return tokens;
}

// The trailing whitespace-delimited token of `query` (the partial the user is
// typing at the caret). Empty when `query` ends in whitespace.
export function trailingToken(value: string): string {
  const match = value.match(/(\S+)$/);
  return match ? match[1] : "";
}

// Whether the trailing token of `raw` is an un-committed field-operator value
// (`app:…`/`source:…`/`date:…`/`after:…`/`before:…`). Pure; used both as the
// backend gate (a half-typed value never reaches `search_capture`) and as the
// basis for the Filter Value List context. Because `trailingToken` returns ""
// once `raw` ends in whitespace, a committed `app:Safari ` is NOT a partial →
// false, so committing/abandoning the value re-opens the backend.
export function isTrailingOperatorPartial(raw: string): boolean {
  return /^(app|source|date|after|before):/i.test(trailingToken(raw));
}

// Quote an operator value when it contains whitespace (or is empty) so the
// backend tokenizer keeps it as one token, e.g. `app:"Google Chrome"`. Bare
// single-word values pass through unquoted (`app:Safari`, `app:com.apple.Safari`).
export function quoteOperatorValue(value: string): string {
  return /\s/.test(value) || value.length === 0 ? `"${value}"` : value;
}

// Format a Date as a local `YYYY-MM-DD` day, the form the backend
// `after:`/`before:` parser accepts (resolve_point_date). Uses local calendar
// fields (not toISOString, which would shift across the UTC boundary).
export function toOperatorDay(d: Date): string {
  const year = d.getFullYear().toString().padStart(4, "0");
  const month = (d.getMonth() + 1).toString().padStart(2, "0");
  const day = d.getDate().toString().padStart(2, "0");
  return `${year}-${month}-${day}`;
}

// Quote an app name that contains whitespace so `app:"Google Chrome"` parses as
// one token; a single-word name is emitted bare (`app:Safari`).
export function appOperatorToken(name: string): string {
  return name.includes(" ") ? `app:"${name}"` : `app:${name}`;
}

// Parse a backend date/datetime string. Tolerates "YYYY-MM-DD HH:MM:SS" (space
// separator) by normalizing to ISO-ish form, matching SearchResultCard.
export function parseToolDate(value: string): Date | null {
  const normalized = value.includes("T") ? value : value.replace(" ", "T");
  const d = new Date(normalized);
  return isNaN(d.getTime()) ? null : d;
}

export function isSameCalendarDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

export function shortDate(d: Date): string {
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
