// Residual-query context highlighting for the Quick Recall detail pane.
// Pure (bun-tested): search results carry NO per-context match offsets, so the
// pane highlights client-side by scanning the fetched OCR/transcript text for
// the backend's `residualQuery` terms (the free text left after filter
// operators were stripped). Substring matching is deliberate — it mirrors the
// backend snippet marks, which highlight inside words ("<mark>Stripe</mark>Event").

export type HighlightSegment = { text: string; marked: boolean };

// Split a residual query into highlightable terms: whitespace-tokenized,
// surrounding punctuation/quotes stripped, lowercased, deduped, and sorted
// longest-first so the regex alternation prefers the longer term when terms
// overlap ("webhook" beats "web" at the same position). Terms shorter than two
// characters are dropped (single letters mark half the text).
export function residualTerms(residualQuery: string): string[] {
  const terms = new Set<string>();
  for (const raw of residualQuery.split(/\s+/)) {
    const term = raw
      .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "")
      .toLowerCase();
    if (term.length >= 2) {
      terms.add(term);
    }
  }
  return [...terms].sort((a, b) => b.length - a.length);
}

function escapeRegExp(term: string): string {
  return term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Split `text` into ordered segments whose `marked` runs are case-insensitive
// occurrences of any term. Concatenating the segments always reproduces the
// input exactly. No terms (or a foundByMeaning result whose terms never
// appear) degrades to a single unmarked segment — the pane renders plain text.
export function highlightSegments(
  text: string,
  terms: string[],
): HighlightSegment[] {
  if (text.length === 0) {
    return [];
  }
  if (terms.length === 0) {
    return [{ text, marked: false }];
  }
  const pattern = new RegExp(terms.map(escapeRegExp).join("|"), "giu");
  const segments: HighlightSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    if (match[0].length === 0) {
      continue;
    }
    if (match.index > last) {
      segments.push({ text: text.slice(last, match.index), marked: false });
    }
    segments.push({ text: match[0], marked: true });
    last = match.index + match[0].length;
  }
  if (last < text.length) {
    segments.push({ text: text.slice(last), marked: false });
  }
  return segments;
}
