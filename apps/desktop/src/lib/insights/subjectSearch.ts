// subjectSearch.ts — pure whole-word, IDF-weighted ranking for the Subjects
// search box. No Svelte, no I/O — just data transforms so it can be unit-tested
// in isolation and reused across the Subjects sub-surfaces.
//
// Ports the relevance approach the backend already uses for `recall_context`
// (crates/app-infra/src/brokered_access.rs): tokenize → light stem → whole-word
// overlap weighted by inverse document frequency, so a rare query word outweighs
// a common one and "cat" matches "cat" but not "category". A subject's NAME is
// weighted above its conclusion statements so typing "apple" ranks the Apple
// subject above one that merely mentions apples in a belief.

// Trivial words dropped from the query so they don't dominate the overlap.
// Mirrors RECALL_STOPWORDS in brokered_access.rs.
const STOPWORDS = new Set<string>([
  "the", "and", "for", "are", "was", "were", "that", "this", "with", "what",
  "when", "where", "who", "why", "how", "did", "does", "have", "has", "had",
  "you", "your", "they", "them", "from", "about", "into", "over", "been",
  "being", "she", "her", "his", "him", "their", "our", "can", "could", "would",
  "should", "will", "shall", "may", "might", "any", "all", "some",
]);

// UTF-8 byte length of a string. The Rust port gates tokens on `str::len()`
// (byte length), so a CJK character — 1 UTF-16 unit but 3 UTF-8 bytes — counts
// toward the >= 3 threshold the same way it does on the backend. Using JS
// `.length` (UTF-16 units) instead would silently drop single/short CJK queries.
const encoder = new TextEncoder();
function byteLength(s: string): number {
  return encoder.encode(s).length;
}

// Extra weight applied to a token that hits the subject NAME (on top of the
// base overlap, which already counts the name as part of the document). ~1.0
// makes a name hit count roughly double a statement-only hit.
const NAME_BOOST = 1.0;

/** Cheap, hand-rolled English suffix stripper — NOT a real stemmer, just a
 *  lexical-gap reducer applied identically to query tokens and corpus words so
 *  "running"~"run", "tests"~"test", "quickly"~"quick" collapse to a shared key.
 *  It only has to be *consistent*, not produce a dictionary stem. Port of
 *  `recall_stem` in brokered_access.rs. */
export function stem(word: string): string {
  // (suffix, min length of the FULL word to apply). Longer suffixes first so
  // `-ing` wins over `-s`. The min-length guards keep very short words intact.
  const RULES: [string, number][] = [
    ["ing", 6],
    ["edly", 7],
    ["ied", 5],
    ["ed", 5],
    ["ly", 5],
    ["ies", 5],
    ["es", 5],
    ["s", 4],
  ];

  // Pass 1: strip the first matching suffix (if a >= 3-char stem survives).
  let stemmed = word;
  for (const [suffix, minLen] of RULES) {
    if (word.length >= minLen && word.endsWith(suffix)) {
      const candidate = word.slice(0, word.length - suffix.length);
      if (candidate.length >= 3) {
        stemmed = candidate;
        break;
      }
    }
  }

  let end = stemmed.length;

  // Pass 2: collapse a doubled final consonant ("runn" -> "run").
  if (end >= 2) {
    const last = stemmed[end - 1];
    const prev = stemmed[end - 2];
    const isConsonant = /[a-z]/.test(last) && !"aeiou".includes(last);
    if (last === prev && isConsonant && end - 1 >= 3) {
      end -= 1;
    }
  }

  // Pass 3: drop a single silent terminal `e` ("code" -> "cod") so the
  // un-suffixed form lines up with the suffixed one. Keep >= 3 chars.
  if (end >= 4 && stemmed[end - 1] === "e") {
    end -= 1;
  }

  return stemmed.slice(0, end);
}

/** Lowercase + tokenize the query into stemmed whole-word keys (>= 3 UTF-8
 *  bytes, punctuation stripped), dropping stopwords and de-duplicating so a
 *  repeated query word can't inflate the score. Empty when the query has no
 *  usable tokens. Port of `recall_query_tokens` — the length gate uses UTF-8
 *  byte length to match the Rust `str::len()` semantics (so a single CJK char,
 *  3 bytes, clears the threshold). */
export function queryTokens(query: string): string[] {
  const tokens: string[] = [];
  // Split on anything that isn't a Unicode letter/number so non-ASCII subject
  // names (accents, CJK, etc.) tokenize too — mirrors the Unicode-aware backend.
  for (const raw of query.split(/[^\p{L}\p{N}]+/u)) {
    const word = raw.toLowerCase();
    if (byteLength(word) < 3 || STOPWORDS.has(word)) continue;
    const stemmed = stem(word);
    if (!tokens.includes(stemmed)) tokens.push(stemmed);
  }
  return tokens;
}

/** Split text into the same lowercased, stemmed whole-word keys (>= 3 UTF-8
 *  bytes) the query is normalized to, so the two sides compare like-for-like.
 *  Port of `recall_doc_words` — the length gate uses UTF-8 byte length to match
 *  the Rust `str::len()` semantics. */
function docWords(text: string): Set<string> {
  const out = new Set<string>();
  for (const raw of text.split(/[^\p{L}\p{N}]+/u)) {
    if (byteLength(raw) < 3) continue;
    out.add(stem(raw.toLowerCase()));
  }
  return out;
}

/** IDF-style weight for a token matching `df` of `n` documents: rarer tokens
 *  (low `df`) outweigh common ones. Always positive so any match still counts.
 *  Port of `recall_idf_weight`. */
function idfWeight(n: number, df: number): number {
  return Math.log((n + 1) / (df + 1)) + 1;
}

function overlap(
  tokens: string[],
  words: Set<string>,
  df: Map<string, number>,
  n: number,
): number {
  let score = 0;
  for (const token of tokens) {
    if (words.has(token)) score += idfWeight(n, df.get(token) ?? 0);
  }
  return score;
}

/** Minimal shape a searchable subject row exposes. A SubjectRow satisfies it. */
export interface SubjectSearchable {
  subject: string;
  conclusions: { statement: string }[];
}

/** Filter + rank `rows` against `query`, keeping only rows that match at least
 *  one query token (score > 0), ordered by relevance descending. Ties keep the
 *  input order (so pass already-sorted rows — e.g. confidence desc — and equal
 *  scores fall back to that). An empty/whitespace query returns `rows`
 *  unchanged so callers can treat "no search" as a passthrough. */
export function rankSubjects<T extends SubjectSearchable>(
  rows: T[],
  query: string,
): T[] {
  const tokens = queryTokens(query);
  if (tokens.length === 0) return rows;

  // Pre-split each row once: name words, and the full document (name + all
  // statements) used for both df and the base overlap.
  const prepped = rows.map((row) => {
    const nameWords = docWords(row.subject);
    const docText = `${row.subject} ${row.conclusions
      .map((c) => c.statement)
      .join(" ")}`;
    return { row, nameWords, allWords: docWords(docText) };
  });

  // Document frequency per query token across the candidate set (port of
  // `recall_document_frequencies`), so rare terms weigh more.
  const n = prepped.length;
  const df = new Map<string, number>();
  for (const token of tokens) {
    let count = 0;
    for (const p of prepped) if (p.allWords.has(token)) count += 1;
    df.set(token, count);
  }

  const scored: { row: T; score: number; index: number }[] = [];
  prepped.forEach((p, index) => {
    const base = overlap(tokens, p.allWords, df, n);
    if (base <= 0) return;
    const nameBonus = NAME_BOOST * overlap(tokens, p.nameWords, df, n);
    scored.push({ row: p.row, score: base + nameBonus, index });
  });

  // Score desc; ties broken by original input order (stable intent made explicit
  // so it survives engines without a stable Array.sort).
  scored.sort((a, b) => b.score - a.score || a.index - b.index);
  return scored.map((s) => s.row);
}
