// Residual-query context highlighting for Quick Look. Pure (bun-tested):
// search results carry NO per-context match offsets, so Quick Look highlights
// client-side by scanning the fetched OCR/transcript text for the backend's
// `residualQuery` terms (the free text left after filter operators were
// stripped). Whole words only, so "the" never lights up inside "whether".
import { STOPWORDS } from "../insights/subjectSearch";

export type HighlightSegment = { text: string; marked: boolean };

// The residual query's terms, as the backend's match boxes take them
// (`query_terms` in crates/app-infra/src/search/match_boxes.rs): `OR` and
// `-exclusions` dropped, split on anything not a letter or digit, lowercased,
// under two characters dropped — plus the backend's lexical stopwords
// (`lexical.rs`, mirrored in subjectSearch.ts), so a question's "what did the"
// marks nothing. Deduped, longest first so the regex alternation prefers the
// longer term where terms overlap ("webhook" beats "web").
export function residualTerms(residualQuery: string): string[] {
  const terms = new Set<string>();
  for (const token of residualQuery.split(/\s+/)) {
    if (token === "OR" || token.startsWith("-")) continue;
    for (const word of token.toLowerCase().split(/[^\p{L}\p{N}]+/u)) {
      if (word.length >= 2 && !STOPWORDS.has(word)) terms.add(word);
    }
  }
  return [...terms].sort((a, b) => b.length - a.length);
}

function escapeRegExp(term: string): string {
  return term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Split `text` into ordered segments whose `marked` runs are case-insensitive
// whole-word occurrences of any term (not touching another letter or digit).
// Concatenating the segments always reproduces the input exactly. No terms (or a foundByMeaning result whose terms never
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
  const pattern = new RegExp(
    `(?<![\\p{L}\\p{N}])(?:${terms.map(escapeRegExp).join("|")})(?![\\p{L}\\p{N}])`,
    "giu",
  );
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
