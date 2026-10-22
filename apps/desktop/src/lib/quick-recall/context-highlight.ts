// Residual-query context highlighting for Quick Look. Pure (bun-tested):
// search results carry NO per-context match offsets, so Quick Look highlights
// client-side by scanning the fetched OCR/transcript text for the backend's
// `residualQuery` terms (the free text left after filter operators were
// stripped). Whole words only, so "web" never lights up inside "website".

export type HighlightSegment = { text: string; marked: boolean };

// Filler words search neither matches nor marks. Mirrors SEARCH_STOPWORDS in
// crates/app-infra/src/search/stopwords.rs — keep the two in sync.
const SEARCH_STOPWORDS = new Set([
  "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "can", "could",
  "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "him", "his", "how",
  "i", "if", "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "over",
  "she", "should", "so", "some", "than", "that", "the", "their", "them", "then", "they",
  "this", "to", "was", "we", "were", "what", "when", "where", "who", "why", "will", "with",
  "would", "you", "your", "about", "all", "any", "may", "might", "shall",
]);

const wordsOf = (text: string) => text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);

type Token = { text: string; quoted: boolean };
const isPositive = (t: Token) => t.quoted || (t.text !== "OR" && !t.text.startsWith("-"));
const isDroppable = (t: Token) =>
  !t.quoted &&
  isPositive(t) &&
  !t.text.endsWith("*") &&
  wordsOf(t.text).length > 0 &&
  wordsOf(t.text).every((w) => SEARCH_STOPWORDS.has(w));

// The residual query's terms, exactly as the backend matches them
// (`ParsedQuery::match_terms`, crates/app-infra/src/search/query.rs): quote-
// aware tokens, `OR` and `-exclusions` dropped, filler stopwords dropped per
// `OR` group unless the group is ALL filler (quoted words always stay), split
// on anything not a letter or digit, lowercased, under two characters dropped.
// Deduped, longest first so the regex alternation prefers the longer term
// where terms overlap ("webhook" beats "web").
export function residualTerms(residualQuery: string): string[] {
  const groups: Token[][] = [[]];
  for (const m of residualQuery.matchAll(/"((?:[^"]|"")*)"|\S+/g)) {
    const token = m[1] !== undefined ? { text: m[1], quoted: true } : { text: m[0], quoted: false };
    if (!token.quoted && token.text === "OR") groups.push([]);
    else groups[groups.length - 1].push(token);
  }
  const terms = new Set<string>();
  for (const group of groups) {
    const drop = group.some((t) => isPositive(t) && !isDroppable(t));
    for (const token of group) {
      if (!isPositive(token) || (drop && isDroppable(token))) continue;
      for (const word of wordsOf(token.text)) if ([...word].length >= 2) terms.add(word);
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
