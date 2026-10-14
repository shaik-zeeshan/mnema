//! Near-duplicate **Conclusion** detection within one **Subject**, plus the
//! startup heal that collapses the paraphrase copies already stored.
//!
//! ADR 0043 made the distiller the matcher: it is shown each candidate subject's
//! conclusions and cites `reinforces_id`. On real data it still re-emits the same
//! belief reworded as "new" (one subject held 44 paraphrases of one sentence), so
//! a code-side guard now backs it up. Two rules, both scoped to ONE subject and
//! never across subjects:
//!
//! - **Lexical** (this module, model-free): stemmed content-token Jaccard
//!   ≥ [`LEXICAL_DUPLICATE_JACCARD`] after dropping stopwords and frequency/hedge
//!   filler. It is the only rule the startup heal uses, because the heal DELETES
//!   rows and must work with no embedding model installed.
//! - **Embedding** (desktop layer, `user_context::conclusion_guard`): cosine
//!   against the subject's existing statements when a Semantic Search model is
//!   installed. app-infra stays embedding-free.
//!
//! Both rules first require [`statements_compatible`]: same negation polarity,
//! same numbers, same time-of-day/week words. Those are exactly the one-word swaps
//! that score as paraphrases but are different beliefs ("plays Genshin" vs
//! "stopped playing Genshin", "works out 3×/week" vs "5×/week", "checks Slack in
//! the morning" vs "in the evening").

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use sqlx::{Row, Sqlite};

use super::store::UserContextStore;
use crate::lexical::{query_tokens, stem};
use crate::Result;

/// Minimum stemmed content-token Jaccard for two statements of the same subject
/// to count as one belief. A one-word swap in a statement of `n` content tokens
/// scores `(n-1)/(n+1)`, so 0.8 lets a swap through only from 9 content tokens
/// up — one-sentence beliefs rarely have that many — while a statement that only
/// adds/drops filler ("frequently", "several times") scores 1.0. Measured on the
/// fixtures in the tests below: every distinct same-subject pair ≤ 0.67, the
/// filler-only paraphrases 1.0.
pub const LEXICAL_DUPLICATE_JACCARD: f64 = 0.8;

/// Frequency/hedge filler the distiller sprinkles differently on every rewrite of
/// the same belief. Dropped (after stemming) before comparing. Stopwords are
/// already dropped by [`query_tokens`].
#[rustfmt::skip]
const FILLER: &[&str] = &[
    "user", "users", "regularly", "frequently", "often", "typically", "usually", "routinely",
    "consistently", "repeatedly", "actively", "tends", "appears", "seems", "likely", "multiple",
    "several", "many", "various", "times", "throughout", "during", "via", "using", "uses",
    "use", "also", "habit", "routine", "each", "every", "per",
];

/// Negation/reversal words: a statement containing one is never the same belief as
/// one that doesn't ("plays X" vs "no longer plays X" / "stopped playing X").
#[rustfmt::skip]
const NEGATIONS: &[&str] = &[
    "not", "no", "never", "stopped", "stop", "stops", "quit", "quits", "rarely", "seldom",
    "without", "longer", "less", "fewer", "dropped", "abandoned", "cannot",
    // Contractions split on a curly apostrophe ("doesn’t" → "doesn", "t").
    "isn", "aren", "wasn", "weren", "doesn", "didn", "don", "hasn", "haven", "won",
];

/// Time-of-day/week qualifiers: routines that differ only in WHEN are distinct
/// beliefs, and embeddings score them ~0.97 alike.
#[rustfmt::skip]
const TIME_WORDS: &[&str] = &[
    "morning", "mornings", "afternoon", "afternoons", "evening", "evenings", "night", "nights",
    "nightly", "overnight", "weekend", "weekends", "weekday", "weekdays", "monday", "tuesday",
    "wednesday", "thursday", "friday", "saturday", "sunday",
];

fn raw_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Stemmed content tokens of a statement: [`query_tokens`] minus [`FILLER`].
fn statement_tokens(statement: &str) -> HashSet<String> {
    static FILLER_STEMS: LazyLock<HashSet<String>> =
        LazyLock::new(|| FILLER.iter().map(|word| stem(word)).collect());
    query_tokens(statement)
        .into_iter()
        .filter(|token| !FILLER_STEMS.contains(token))
        .collect()
}

/// The qualifiers two statements must agree on before any similarity counts:
/// negation polarity, the set of numbers, and the set of time words.
fn qualifiers(statement: &str) -> (bool, Vec<String>) {
    let mut negated = false;
    let mut words: Vec<String> = Vec::new();
    for word in raw_words(statement) {
        if NEGATIONS.contains(&word.as_str()) || word.ends_with("n't") {
            negated = true;
        } else if word.chars().all(|ch| ch.is_ascii_digit()) || TIME_WORDS.contains(&word.as_str())
        {
            words.push(word.trim_end_matches('s').to_string());
        }
    }
    words.sort();
    words.dedup();
    (negated, words)
}

/// Whether two statements agree on negation, numbers and time words — the
/// precondition for both the lexical and the embedding duplicate rule.
pub fn statements_compatible(a: &str, b: &str) -> bool {
    qualifiers(a) == qualifiers(b)
}

/// One statement pre-parsed for comparison, so a scan over a large subject
/// tokenizes each row once instead of once per pair.
struct Fingerprint {
    exact: String,
    tokens: HashSet<String>,
    qualifiers: (bool, Vec<String>),
}

impl Fingerprint {
    fn of(statement: &str) -> Self {
        Self {
            exact: statement.trim().to_lowercase(),
            tokens: statement_tokens(statement),
            qualifiers: qualifiers(statement),
        }
    }

    /// Content-token Jaccard; 0.0 when either side has no tokens.
    fn jaccard(&self, other: &Self) -> f64 {
        if self.tokens.is_empty() || other.tokens.is_empty() {
            return 0.0;
        }
        let shared = self.tokens.intersection(&other.tokens).count();
        shared as f64 / (self.tokens.len() + other.tokens.len() - shared) as f64
    }

    /// The lexical duplicate rule: case-insensitive equal, or same qualifiers
    /// with a content-token Jaccard ≥ [`LEXICAL_DUPLICATE_JACCARD`].
    fn duplicates(&self, other: &Self) -> bool {
        self.exact == other.exact
            || (self.qualifiers == other.qualifiers
                && self.jaccard(other) >= LEXICAL_DUPLICATE_JACCARD)
    }
}

/// Whether `a` and `b` (same subject) are one belief under the lexical rule.
pub fn is_lexical_duplicate(a: &str, b: &str) -> bool {
    Fingerprint::of(a).duplicates(&Fingerprint::of(b))
}

/// The id of the existing conclusion `statement` lexically duplicates, if any —
/// the most similar one when several qualify (tie → lowest id). `existing` must
/// all be the SAME subject as `statement`; this never decides subject identity.
pub fn find_lexical_duplicate<'a>(
    statement: &str,
    existing: impl IntoIterator<Item = (i64, &'a str)>,
) -> Option<i64> {
    let probe = Fingerprint::of(statement);
    existing
        .into_iter()
        .filter_map(|(id, other)| {
            let other = Fingerprint::of(other);
            probe
                .duplicates(&other)
                .then(|| (id, probe.jaccard(&other)))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(id, _)| id)
}

/// What one [`UserContextStore::dedupe_near_duplicate_conclusions`] pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConclusionDedupSummary {
    pub subjects_touched: u64,
    pub conclusions_removed: u64,
}

/// One live conclusion row as the heal sees it.
struct DedupRow {
    id: i64,
    statement: String,
    pinned: bool,
}

/// Greedy grouping of one subject's rows (already ordered strongest-first:
/// pinned, then confidence desc, then id asc). Each row joins the first keeper it
/// duplicates, else becomes a keeper itself. Comparing only against keepers — not
/// chaining through members — keeps A~B~C drift from fusing A with a different C.
/// Pinned rows are never removed (always keepers). Returns `(copy, keeper)` pairs.
/// ponytail: O(rows × keepers) per subject — ~140 ms (release) for a 1,600-row
/// subject with no duplicates; add a token inverted index if subjects grow far past that.
fn plan_merges(rows: &[DedupRow]) -> Vec<(i64, i64)> {
    let mut keepers: Vec<(i64, Fingerprint)> = Vec::new();
    let mut merges = Vec::new();
    for row in rows {
        let fingerprint = Fingerprint::of(&row.statement);
        let keeper = if row.pinned {
            None
        } else {
            keepers
                .iter()
                .find(|(_, keeper)| keeper.duplicates(&fingerprint))
        };
        match keeper {
            Some((keeper_id, _)) => merges.push((row.id, *keeper_id)),
            None => keepers.push((row.id, fingerprint)),
        }
    }
    merges
}

/// Live (not dismissed/superseded) rows, strongest first — the keeper order.
const LIVE_ROWS_SQL: &str = "SELECT id, subject, statement, COALESCE(pinned, 0) AS pinned \
     FROM user_context_conclusions \
     WHERE status NOT IN ('dismissed', 'superseded') {filter} \
     ORDER BY COALESCE(pinned, 0) DESC, confidence DESC, id ASC";

impl UserContextStore {
    /// Every live conclusion of `subject` (case-insensitive) as `(id, statement)`,
    /// strongest first. The duplicate guard's comparison set.
    pub async fn list_live_statements_for_subject(
        &self,
        subject: &str,
    ) -> Result<Vec<(i64, String)>> {
        let rows =
            sqlx::query(&LIVE_ROWS_SQL.replace("{filter}", "AND subject = ?1 COLLATE NOCASE"))
                .bind(subject)
                .fetch_all(self.db.read())
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get("id"), row.get("statement")))
            .collect())
    }

    /// Startup heal: collapse lexical near-duplicate conclusions within each
    /// subject onto the strongest copy (pinned first, then highest confidence,
    /// tie → lowest id). Per subject, in ONE transaction: the copies' evidence is
    /// re-pointed to the keeper (`INSERT OR IGNORE` on the
    /// `UNIQUE(conclusion_id, activity_id)` so shared activities are not
    /// duplicated), any `superseded_by` pointing at a copy is re-pointed, the
    /// keeper takes the highest confidence, latest `last_supported_at_ms` and
    /// earliest `formed_at_ms`, and the copies are deleted. Their Confidence
    /// History is deleted with them, not merged: interleaving a copy's points into
    /// the keeper's series would draw a zig-zag trajectory neither belief had.
    ///
    /// Idempotent (keepers are pairwise non-duplicate, so a re-run plans nothing)
    /// and cheap when clean: one read of the live rows, no write transaction
    /// unless a subject has something to merge. Touches only `user_context_*`
    /// conclusion tables (ADR 0029 is about retention; nothing here cascades into
    /// or out of capture data).
    pub async fn dedupe_near_duplicate_conclusions(&self) -> Result<ConclusionDedupSummary> {
        let rows = sqlx::query(&LIVE_ROWS_SQL.replace("{filter}", ""))
            .fetch_all(self.db.read())
            .await?;
        let mut by_subject: HashMap<String, Vec<DedupRow>> = HashMap::new();
        for row in rows {
            by_subject
                // ASCII fold = SQLite NOCASE, the store's subject identity.
                .entry(row.get::<String, _>("subject").to_ascii_lowercase())
                .or_default()
                .push(DedupRow {
                    id: row.get("id"),
                    statement: row.get("statement"),
                    pinned: row.get::<i64, _>("pinned") != 0,
                });
        }

        let mut summary = ConclusionDedupSummary::default();
        for (subject, rows) in by_subject {
            if rows.len() < 2 || plan_merges(&rows).is_empty() {
                continue;
            }
            // Re-plan inside the write txn: a dismiss/pin may have landed since the
            // snapshot read above.
            let mut transaction = self.db.begin_write().await?;
            let fresh: Vec<DedupRow> =
                sqlx::query(&LIVE_ROWS_SQL.replace("{filter}", "AND subject = ?1 COLLATE NOCASE"))
                    .bind(&subject)
                    .fetch_all(&mut *transaction)
                    .await?
                    .into_iter()
                    .map(|row| DedupRow {
                        id: row.get("id"),
                        statement: row.get("statement"),
                        pinned: row.get::<i64, _>("pinned") != 0,
                    })
                    .collect();
            let merges = plan_merges(&fresh);
            for (copy, keeper) in &merges {
                merge_into_keeper(&mut transaction, *copy, *keeper).await?;
            }
            transaction.commit().await?;
            if !merges.is_empty() {
                summary.subjects_touched += 1;
                summary.conclusions_removed += merges.len() as u64;
            }
        }
        Ok(summary)
    }
}

/// Fold one duplicate row into its keeper, inside the caller's transaction.
async fn merge_into_keeper(
    transaction: &mut sqlx::Transaction<'static, Sqlite>,
    copy: i64,
    keeper: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO user_context_conclusion_evidence \
            (conclusion_id, activity_id, stance, created_at_ms) \
         SELECT ?2, activity_id, stance, created_at_ms \
         FROM user_context_conclusion_evidence WHERE conclusion_id = ?1",
    )
    .bind(copy)
    .bind(keeper)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "UPDATE user_context_conclusions SET \
            confidence = MAX(confidence, \
                (SELECT confidence FROM user_context_conclusions WHERE id = ?1)), \
            last_supported_at_ms = MAX(last_supported_at_ms, \
                (SELECT last_supported_at_ms FROM user_context_conclusions WHERE id = ?1)), \
            formed_at_ms = MIN(formed_at_ms, \
                (SELECT formed_at_ms FROM user_context_conclusions WHERE id = ?1)) \
         WHERE id = ?2",
    )
    .bind(copy)
    .bind(keeper)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE user_context_conclusions SET superseded_by = ?2 WHERE superseded_by = ?1")
        .bind(copy)
        .bind(keeper)
        .execute(&mut **transaction)
        .await?;
    // Explicit child deletes so the heal never depends on `PRAGMA foreign_keys`.
    for sql in [
        "DELETE FROM user_context_conclusion_evidence WHERE conclusion_id = ?1",
        "DELETE FROM user_context_confidence_history WHERE conclusion_id = ?1",
        "DELETE FROM user_context_conclusions WHERE id = ?1",
    ] {
        sqlx::query(sql)
            .bind(copy)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "conclusion_dedup_tests.rs"]
mod tests;
