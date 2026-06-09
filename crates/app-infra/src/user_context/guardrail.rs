//! Sensitive Category Guardrail (issue #96, [ADR 0030]).
//!
//! A hard policy that keeps **Conclusion** values (and **Activity Category** /
//! **Focus Classification** labels) in off-limits inference categories — health
//! and mental health, sexual orientation, religion, politics, and similar
//! protected/intimate domains — out of the **User Context** dossier. The same
//! **Reasoning Engine** that concludes "likes Apple" will, pointed at a person's
//! whole digital life, just as readily conclude "appears depressed," "is probably
//! pregnant," or "leans politically conservative"; a grounded, persisted
//! inference of that kind is the most incriminating file on the user's disk and
//! cuts hard against Mnema's conservative, app-only privacy posture.
//!
//! Enforced **two ways**, because neither alone is enough (ADR 0030):
//!
//! - a **soft** instruction ([`SENSITIVE_GUARDRAIL_INSTRUCTION`]) prepended to the
//!   Conclusion-distillation preamble, telling the engine not to form such
//!   conclusions in the first place; and
//! - a **hard**, deterministic post-filter ([`is_sensitive`]) that drops any
//!   Conclusion whose **Subject** / statement lands in a sensitive category
//!   *before it is ever persisted or surfaced* — the backstop for when the model
//!   ignores the instruction.
//!
//! The guardrail is **suppressed by default and not user-enableable in v1**
//! (there is no "infer my mental health" toggle), and it deliberately errs toward
//! **over-suppression**: it would rather false-suppress a benign conclusion that
//! brushes a sensitive category than false-surface a real sensitive inference.
//! Because [`is_sensitive`] runs at *derivation* time, sensitive Conclusions
//! never enter the **Encrypted Capture Index**, so the `recall_context` broker
//! tool that **Ask AI** uses physically cannot return them — guardrailing is
//! **not** re-implemented at the broker boundary.
//!
//! This module is **pure** (no I/O, no clock): match logic only, so the policy is
//! unit-tested in isolation.
//!
//! [ADR 0030]: ../../../../docs/adr/0030-user-context-sensitive-category-guardrail.md

/// The **soft** guardrail instruction — the part of the policy the
/// **Reasoning Engine** sees. Prepended to the Conclusion-distillation preamble
/// (and noted in the Activity preamble's category guidance) so the engine is told
/// not to form sensitive conclusions in the first place. This is necessary but
/// **not sufficient**: an LLM told to avoid a category will sometimes do it
/// anyway, which is why [`is_sensitive`] is the hard backstop.
pub const SENSITIVE_GUARDRAIL_INSTRUCTION: &str = "PRIVACY GUARDRAIL (mandatory): Do NOT form any \
Conclusion, and do NOT assign any Activity Category or Focus label, that is about — or that lets a \
reader infer — the user's health or mental health (including mood, diagnoses, medication, \
pregnancy, addiction, or body image), sexual orientation or sexuality, religion or religious \
practice, political views or affiliation, or other protected or intimate domains such as \
race/ethnicity, disability, immigration status, or financial distress. These categories are \
off-limits regardless of how strongly the captures seem to support them. When a possible \
conclusion brushes any of these areas, or you are even slightly unsure, simply do not form it — \
silently omit it rather than hedging. Prefer omission to inference here.";

/// The off-limits inference categories of the Sensitive Category Guardrail. Used
/// to make [`is_sensitive`] explainable in tests; callers only need the boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitiveCategory {
    /// Health and mental health: diagnoses, medication, mood, pregnancy,
    /// addiction, body image, etc.
    Health,
    /// Sexual orientation / sexuality / gender identity.
    SexualOrientation,
    /// Religion and religious practice.
    Religion,
    /// Politics and political affiliation.
    Politics,
    /// Other protected/intimate domains: race/ethnicity, disability, immigration
    /// status, clear financial distress.
    OtherProtected,
}

/// The hard post-filter: returns `true` when the Conclusion (its `subject` +
/// `statement`) should be **DROPPED** as landing in a sensitive category. Wraps
/// [`category_of`].
///
/// Matching is **case-insensitive** and **whole-word / whole-phrase** (word
/// boundaries) over `subject` and `statement` combined, so substrings inside
/// benign words do not trip the filter ("god" must not match "good", "race" must
/// not match "embrace", "trans" must not match "transaction"). The lists below
/// bias toward **over-suppression** (ADR 0030) while keeping plainly-benign
/// conclusions like "Prefers async communication" or "Is in a Rust learning
/// phase" clear of every trigger.
pub fn is_sensitive(subject: &str, statement: &str) -> bool {
    category_of(subject, statement).is_some()
}

/// Like [`is_sensitive`] but reports *which* category tripped (or `None`). The
/// first matching category wins; the boolean wrapper does not care which.
pub fn category_of(subject: &str, statement: &str) -> Option<SensitiveCategory> {
    // Normalize once: lowercase, and reduce every non-alphanumeric run to a single
    // space so phrase matching ("mental health", "left-wing", "weight loss") is a
    // simple space-padded substring test that still respects word boundaries.
    let haystack = normalize(&format!("{subject} {statement}"));

    if matches_any(&haystack, HEALTH_TERMS) {
        return Some(SensitiveCategory::Health);
    }
    if matches_any(&haystack, SEXUAL_ORIENTATION_TERMS) {
        return Some(SensitiveCategory::SexualOrientation);
    }
    if matches_any(&haystack, RELIGION_TERMS) {
        return Some(SensitiveCategory::Religion);
    }
    if matches_any(&haystack, POLITICS_TERMS) {
        return Some(SensitiveCategory::Politics);
    }
    if matches_any(&haystack, OTHER_PROTECTED_TERMS) {
        return Some(SensitiveCategory::OtherProtected);
    }
    None
}

/// Lowercase the text and collapse every run of non-alphanumeric characters into a
/// single ASCII space, with a leading and trailing space added. The space padding
/// lets [`matches_any`] do whole-word matching with a plain ` term `-bounded
/// `contains`, so "god" never matches inside "good" and "race" never matches
/// inside "embrace", while multi-word phrases like "mental health" still match.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(' ');
    let mut prev_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

/// Whether the (already [`normalize`]d, space-padded) haystack contains any of the
/// given terms as a whole word/phrase. Each term is itself normalized and matched
/// space-bounded so it cannot match a substring inside a larger word.
fn matches_any(haystack: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| {
        let needle = normalize(term);
        // `needle` is already ` term ` padded; a plain contains is whole-word.
        haystack.contains(&needle)
    })
}

// === Off-limits term lists (whole-word; bias to over-suppression) ============
//
// Each entry is matched as a whole word or whole phrase (see `matches_any`).
// Precision notes inline mark terms deliberately kept narrow so benign
// conclusions ("interested in Apple", "Rust learning phase", "evenings gaming")
// do not trip.

/// Health & mental health: diagnoses, medication, mood, pregnancy, addiction,
/// and body-image/dieting senses.
const HEALTH_TERMS: &[&str] = &[
    "depressed",
    "depression",
    "anxiety",
    "anxious",
    "therapy",
    "therapist",
    "diagnosis",
    "diagnosed",
    "illness",
    "disease",
    "mental health",
    "medication",
    "prescription",
    "pregnant",
    "pregnancy",
    "symptoms",
    "symptom",
    "adhd",
    "bipolar",
    "addiction",
    "addicted",
    "rehab",
    "disorder",
    "suicidal",
    "suicide",
    "self harm",
    // Body-image / dieting sense. "diet"/"dieting" and "weight loss" read as a
    // health/body-image inference; this is an accepted over-suppression source
    // (a benign nutrition interest can trip it — see the unit test).
    "dieting",
    "weight loss",
    "eating disorder",
    "anorexia",
    "bulimia",
];

/// Sexual orientation / sexuality / gender identity.
const SEXUAL_ORIENTATION_TERMS: &[&str] = &[
    "gay",
    "lesbian",
    "bisexual",
    "queer",
    "lgbt",
    "lgbtq",
    "lgbtqia",
    "sexual orientation",
    "sexuality",
    "transgender",
    // "trans" alone is whole-word matched, so "transaction"/"transit"/"transfer"
    // are NOT affected (they are single longer words).
    "trans",
    "heterosexual",
    "homosexual",
    "asexual",
    "pansexual",
];

/// Religion and religious practice.
const RELIGION_TERMS: &[&str] = &[
    "religion",
    "religious",
    "faith",
    "christian",
    "christianity",
    "catholic",
    "muslim",
    "islam",
    "islamic",
    "jewish",
    "judaism",
    "hindu",
    "hinduism",
    "buddhist",
    "buddhism",
    "atheist",
    "atheism",
    "agnostic",
    "church",
    "mosque",
    "synagogue",
    "temple",
    "prayer",
    "praying",
    "worship",
    "spiritual",
];

/// Politics and political affiliation.
const POLITICS_TERMS: &[&str] = &[
    "politics",
    "political",
    "politically",
    "democrat",
    "democratic",
    "republican",
    // "liberal"/"conservative" carry a political stance in this context; matched
    // whole-word so they read as affiliation, not e.g. "conservative estimate"
    // (still a deliberate over-suppression — bias is toward dropping).
    "liberal",
    "conservative",
    "left wing",
    "right wing",
    "election",
    "voting",
    "voter",
    "campaign",
    "ideology",
    "partisan",
    "maga",
    "marxist",
    "socialist",
    "communist",
    "fascist",
];

/// Other protected/intimate domains: race/ethnicity, disability, immigration
/// status, and clear financial-distress phrasings.
const OTHER_PROTECTED_TERMS: &[&str] = &[
    // Race / ethnicity. "race" is whole-word, so "embrace"/"trace"/"racecar"
    // are NOT matched.
    "race",
    "racial",
    "ethnicity",
    "ethnic",
    "immigrant",
    "immigration",
    "undocumented",
    "deportation",
    "asylum",
    "refugee",
    "visa status",
    "green card",
    // Disability.
    "disability",
    "disabled",
    "handicapped",
    // Financial distress (clear phrasings only — plain "money"/"budget" stay
    // benign; only distress reads as protected/intimate).
    "bankruptcy",
    "bankrupt",
    "in debt",
    "debt collector",
    "foreclosure",
    "evicted",
    "eviction",
    "unemployed",
    "laid off",
    "financial distress",
];

#[cfg(test)]
mod tests {
    use super::*;

    // --- The soft instruction is wired and non-empty -----------------------

    #[test]
    fn soft_instruction_names_the_off_limits_domains() {
        let text = SENSITIVE_GUARDRAIL_INSTRUCTION.to_ascii_lowercase();
        assert!(!text.trim().is_empty());
        assert!(text.contains("health"));
        assert!(text.contains("sexual"));
        assert!(text.contains("religion"));
        assert!(text.contains("political"));
        // It must tell the engine to omit when in doubt (over-suppression).
        assert!(text.contains("do not form") || text.contains("not form"));
    }

    // --- ADR 0030 sensitive examples → DROPPED (true) ----------------------

    #[test]
    fn adr_examples_are_suppressed() {
        // ADR 0030 example conclusions, each pointed at a plausible Subject.
        assert!(is_sensitive("mood", "appears depressed lately"));
        assert!(is_sensitive("family planning", "is probably pregnant"));
        assert!(is_sensitive(
            "politics",
            "leans politically conservative"
        ));
        assert!(is_sensitive(
            "religion",
            "is likely exploring a new religion"
        ));
        assert!(is_sensitive("identity", "is gay"));
        assert!(is_sensitive("identity", "is bisexual"));
    }

    #[test]
    fn each_category_is_covered() {
        assert_eq!(
            category_of("mood", "has been feeling anxious and started therapy"),
            Some(SensitiveCategory::Health)
        );
        assert_eq!(
            category_of("identity", "is a lesbian"),
            Some(SensitiveCategory::SexualOrientation)
        );
        assert_eq!(
            category_of("beliefs", "attends church every Sunday"),
            Some(SensitiveCategory::Religion)
        );
        assert_eq!(
            category_of("views", "is a registered democrat"),
            Some(SensitiveCategory::Politics)
        );
        assert_eq!(
            category_of("status", "is an undocumented immigrant"),
            Some(SensitiveCategory::OtherProtected)
        );
        assert_eq!(
            category_of("finances", "is facing bankruptcy and was evicted"),
            Some(SensitiveCategory::OtherProtected)
        );
    }

    // --- Benign conclusions → KEPT (false) ---------------------------------

    #[test]
    fn benign_conclusions_are_not_suppressed() {
        // The precision guardrails called out in the issue.
        assert!(!is_sensitive(
            "Apple",
            "Has been increasingly interested in Apple"
        ));
        assert!(!is_sensitive(
            "async communication",
            "Prefers async communication"
        ));
        assert!(!is_sensitive("Rust", "Is in a Rust learning phase"));
        assert!(!is_sensitive("gaming", "Spends evenings gaming"));
        // A few more plainly-benign work/tech conclusions.
        assert!(!is_sensitive("design", "Cares a lot about clean design"));
        assert!(!is_sensitive(
            "productivity",
            "Tends to batch similar tasks together"
        ));
        assert!(!is_sensitive(
            "Apple",
            "Switched to a new MacBook this month"
        ));
    }

    #[test]
    fn whole_word_matching_avoids_substring_false_positives() {
        // "god" must not match inside "good"; "race" not inside "embrace";
        // "trans" not inside "transaction"; "diet" sense via "dieting" only.
        assert!(!is_sensitive("habits", "is a good and thorough engineer"));
        assert!(!is_sensitive(
            "teamwork",
            "tends to embrace new tooling quickly"
        ));
        assert!(!is_sensitive(
            "finance app",
            "reviews each transaction carefully"
        ));
        assert!(!is_sensitive(
            "commute",
            "uses public transit to get to work"
        ));
        // "election" should not match inside a benign unrelated word — none here,
        // but confirm "selection"/"collection" stay benign (no whole-word hit).
        assert!(!is_sensitive(
            "shopping",
            "spends time on product selection"
        ));
        assert!(!is_sensitive("music", "has a large record collection"));
        // "campaign" is political-list; "champagne"/"campground" must stay benign.
        assert!(!is_sensitive("travel", "enjoys visiting a campground"));
    }

    // --- Deliberate over-suppression blind spot (documented) ---------------

    #[test]
    fn deliberate_over_suppression_of_benign_health_adjacent_terms() {
        // ADR 0030 accepts that the hard filter will sometimes suppress a benign
        // conclusion that merely BRUSHES a sensitive category — "the dossier has
        // a deliberate, invisible blind spot by design." A plain healthy-eating /
        // dieting interest trips the health filter via "dieting"/"weight loss".
        // This returning TRUE is the intended over-suppression, not a bug.
        assert!(is_sensitive(
            "nutrition",
            "has been dieting and tracking weight loss"
        ));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(is_sensitive("MOOD", "APPEARS DEPRESSED"));
        assert!(is_sensitive("Religion", "Is Religious"));
        assert!(is_sensitive("Politics", "Leans Conservative"));
    }
}
