//! Shared lexical-ranking primitives: tokenize → light stem → whole-word,
//! IDF-weighted overlap. Pure text, no embeddings, no I/O.
//!
//! Originally private to [`crate::brokered_access`] (the `recall_*` helpers behind
//! the `recall_context` broker tool). Lifted here so the User Context distillation
//! candidate selector can reuse the SAME ranking to find existing Subject handles
//! that lexically overlap the recent Activity text — a model-free, lag-free
//! complement to the embedding (semantic) candidate leg. The frontend
//! `apps/desktop/src/lib/insights/subjectSearch.ts` is a hand-mirrored TS port of
//! these functions; keep the three in sync.

use std::collections::{HashMap, HashSet};

/// Trivial words dropped from a query so they cannot dominate the overlap. The TS
/// port mirrors this set.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "were", "that", "this", "with", "what", "when", "where",
    "who", "why", "how", "did", "does", "have", "has", "had", "you", "your", "they", "them",
    "from", "about", "into", "over", "been", "being", "she", "her", "his", "him", "their", "our",
    "can", "could", "would", "should", "will", "shall", "may", "might", "any", "all", "some",
];

/// Lowercase, tokenize the query into words (length >= 3, punctuation stripped),
/// dropping trivial stopwords, then **stem** each survivor ([`stem`]) so the
/// matcher is morphology-insensitive ("running" ~ "run"). Empty when the query has
/// no usable tokens. Tokens are de-duplicated so a repeated query word cannot
/// inflate the overlap score.
pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for word in query.split(|ch: char| !ch.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.len() < 3 || STOPWORDS.contains(&word.as_str()) {
            continue;
        }
        let stemmed = stem(&word);
        if !tokens.contains(&stemmed) {
            tokens.push(stemmed);
        }
    }
    tokens
}

/// Cheap, hand-rolled English suffix stripper (#3) — NOT a real stemmer, just a
/// lexical-gap reducer applied identically to query tokens and corpus words so
/// "running"~"run", "coding"~"code", "tests"~"test", "quickly"~"quick" collapse
/// to a shared key. It does NOT try to produce a real dictionary stem; it only
/// has to be *consistent*, so a query word and the corpus word it should match
/// land on the same key.
///
/// Three passes: (1) strip one common suffix (`-ing`, `-edly`, `-ied`, `-ed`,
/// `-ly`, `-ies`, `-es`, `-s`); (2) collapse a doubled final consonant
/// ("runn" -> "run", "stopp" -> "stop") so the `-ing`/`-ed` doubling rule is
/// undone; (3) drop a single silent terminal `e` from whatever remains so the
/// un-suffixed form lines up with the suffixed one ("code" -> "cod" matches
/// "coding" -> "cod"). Guards against over-stemming: a suffix is only stripped
/// when a reasonable stem (>= 3 chars) remains, so short words like
/// "is"/"red"/"bus"/"ring" are left intact. No allocation when nothing changes.
pub(crate) fn stem(word: &str) -> String {
    // Each rule: (suffix, min length of the FULL word to apply). Longer suffixes
    // first so `-ing` wins over `-s`. The min-length guards keep very short words
    // from being gutted.
    const RULES: &[(&str, usize)] = &[
        ("ing", 6),
        ("edly", 7),
        ("ied", 5),
        ("ed", 5),
        ("ly", 5),
        ("ies", 5),
        ("es", 5),
        ("s", 4),
    ];

    // Pass 1: strip the first matching suffix (if a >= 3-char stem survives).
    let mut stem = word;
    for (suffix, min_len) in RULES {
        if word.len() >= *min_len && word.ends_with(suffix) {
            let candidate = &word[..word.len() - suffix.len()];
            if candidate.len() >= 3 {
                stem = candidate;
                break;
            }
        }
    }

    let bytes = stem.as_bytes();
    let mut end = bytes.len();

    // Pass 2: collapse a doubled final consonant ("runn" -> "run").
    if end >= 2 {
        let last = bytes[end - 1];
        let prev = bytes[end - 2];
        let is_consonant = last.is_ascii_alphabetic() && !b"aeiou".contains(&last);
        if last == prev && is_consonant && end - 1 >= 3 {
            end -= 1;
        }
    }

    // Pass 3: drop a single silent terminal `e` ("code" -> "cod") so the
    // un-suffixed form matches the suffixed one. Keep >= 3 chars.
    if end >= 4 && bytes[end - 1] == b'e' {
        end -= 1;
    }

    stem[..end].to_string()
}

/// Split `text` into lowercased, stemmed whole-word keys (length >= 3), the same
/// normalization [`query_tokens`] applies to the query so the two sides compare
/// like-for-like. Used to build per-document word sets for whole-word (#1)
/// matching and IDF (#2) document-frequency.
pub(crate) fn doc_words(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(|word| stem(&word.to_lowercase()))
        .collect()
}

/// IDF-style weight for a token matching `df` of `n` candidate documents: rarer
/// tokens (low `df`) outweigh common ones. `ln((N+1)/(df+1)) + 1`, always
/// positive so any match still counts. (#2)
pub(crate) fn idf_weight(n: usize, df: usize) -> f64 {
    (((n as f64 + 1.0) / (df as f64 + 1.0)).ln()) + 1.0
}

/// Build a token -> document-frequency map over the candidate `docs` (each a
/// pre-split whole-word set), counting only tokens that are actual query tokens.
/// (#2)
pub(crate) fn document_frequencies(
    tokens: &[String],
    docs: &[HashSet<String>],
) -> HashMap<String, usize> {
    let mut df: HashMap<String, usize> = HashMap::new();
    for token in tokens {
        let count = docs.iter().filter(|words| words.contains(token)).count();
        df.insert(token.clone(), count);
    }
    df
}

/// Whole-word (#1), rare-token-weighted (#2) relevance score of `tokens` against
/// a document's pre-split whole-word set `doc_words`: sums the IDF weight of each
/// query token present as a full (stemmed) word. Substring hits no longer count —
/// "cat" matches "cat" but not "category". Returns `0.0` when nothing matches.
pub(crate) fn overlap_score(
    tokens: &[String],
    doc_words: &HashSet<String>,
    df: &HashMap<String, usize>,
    n: usize,
) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    tokens
        .iter()
        .filter(|token| doc_words.contains(*token))
        .map(|token| idf_weight(n, df.get(token).copied().unwrap_or(0)))
        .sum()
}

/// Extra weight for a query token that also hits a candidate's NAME (the handle),
/// on top of the base document overlap. ~1.0 makes a name hit count roughly double
/// a body-only hit. Mirrors `NAME_BOOST` in the frontend `subjectSearch.ts` so a
/// query word in the Subject name outranks one that only appears in a statement.
const NAME_BOOST: f64 = 1.0;

/// Rank `candidates` — each `(handle, body_text)` — against `query` by whole-word,
/// IDF-weighted overlap with a NAME boost, returning the handles that match at
/// least one query token (score > 0), most-relevant first, capped at `limit`.
///
/// The score is the overlap over the full document (handle + body) plus a
/// [`NAME_BOOST`]-scaled overlap over the handle alone, so a Subject whose NAME
/// shares words with the query ranks above one that merely mentions them in a
/// statement. Ties preserve input order, so pass candidates pre-ordered by recency
/// to get a recency tiebreak. Empty when `query` yields no usable tokens (the
/// caller then contributes no lexical candidates). Rust twin of `rankSubjects` in
/// `apps/desktop/src/lib/insights/subjectSearch.ts`.
pub(crate) fn rank_handles_by_overlap(
    query: &str,
    candidates: &[(String, String)],
    limit: usize,
) -> Vec<String> {
    let tokens = query_tokens(query);
    if tokens.is_empty() || candidates.is_empty() || limit == 0 {
        return Vec::new();
    }

    // Pre-split each candidate once: the name (handle) words, and the full document
    // (handle + body) used for both df and the base overlap.
    let prepped: Vec<(&str, HashSet<String>, HashSet<String>)> = candidates
        .iter()
        .map(|(handle, body)| {
            let name_words = doc_words(handle);
            let all_words = doc_words(&format!("{handle} {body}"));
            (handle.as_str(), name_words, all_words)
        })
        .collect();

    let docs: Vec<HashSet<String>> = prepped.iter().map(|(_, _, all)| all.clone()).collect();
    let n = prepped.len();
    let df = document_frequencies(&tokens, &docs);

    let mut scored: Vec<(usize, f64, &str)> = Vec::new();
    for (index, (handle, name_words, all_words)) in prepped.iter().enumerate() {
        let base = overlap_score(&tokens, all_words, &df, n);
        if base <= 0.0 {
            continue;
        }
        let name_bonus = NAME_BOOST * overlap_score(&tokens, name_words, &df, n);
        scored.push((index, base + name_bonus, handle));
    }

    // Score desc; ties broken by original input order (so a recency-ordered
    // candidate list yields a recency tiebreak).
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, _, handle)| handle.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_reworded_duplicate_above_unrelated_subject() {
        // The live bug: a window about watching Marvel Rivals videos must surface
        // the existing "Marvel Rivals / gaming" handle (shared name tokens) ahead
        // of an unrelated subject — purely lexically, no embedding model.
        let candidates = vec![
            ("Marvel Rivals / gaming".to_string(), "Watches gaming clips".to_string()),
            ("async communication".to_string(), "Prefers Slack over meetings".to_string()),
        ];
        let ranked = rank_handles_by_overlap(
            "Watching Marvel Rivals gaming videos on YouTube",
            &candidates,
            10,
        );
        assert_eq!(ranked.first().map(String::as_str), Some("Marvel Rivals / gaming"));
        assert!(!ranked.contains(&"async communication".to_string()));
    }

    #[test]
    fn name_hit_outranks_statement_only_hit() {
        // "Apple" in the NAME beats a subject that only mentions apple in a body.
        let candidates = vec![
            ("Fruit shopping".to_string(), "Bought an apple and a pear".to_string()),
            ("Apple".to_string(), "Interested in the company".to_string()),
        ];
        let ranked = rank_handles_by_overlap("apple", &candidates, 10);
        assert_eq!(ranked.first().map(String::as_str), Some("Apple"));
    }

    #[test]
    fn no_usable_query_tokens_yields_no_candidates() {
        let candidates = vec![("Apple".to_string(), "x".to_string())];
        // All-stopword / too-short query → no tokens → empty (caller adds no
        // lexical leg, falls back to recency + semantic only).
        assert!(rank_handles_by_overlap("the and a", &candidates, 10).is_empty());
    }

    #[test]
    fn caps_and_preserves_recency_order_on_ties() {
        // Equal-scoring matches keep input order, so a recency-ordered input gives a
        // recency tiebreak; the cap keeps only the first `limit`.
        let candidates: Vec<(String, String)> = (0..5)
            .map(|i| (format!("rust topic {i}"), String::new()))
            .collect();
        let ranked = rank_handles_by_overlap("rust", &candidates, 3);
        assert_eq!(
            ranked,
            vec![
                "rust topic 0".to_string(),
                "rust topic 1".to_string(),
                "rust topic 2".to_string(),
            ]
        );
    }
}
