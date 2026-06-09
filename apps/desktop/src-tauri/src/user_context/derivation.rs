//! LLM orchestration for **Activity** derivation (issue #93).
//!
//! Builds a prompt from a redacted [`app_infra::CaptureWindow`], asks the
//! configured [`ai_engine::EngineConfig`] to segment the window into semantic
//! **Activity** episodes (bounded by *intent shifts*, not per-app or per-time
//! slices), and persists each one with its raw-capture evidence via
//! [`app_infra::UserContextStore::insert_activity_with_evidence`].
//!
//! Conclusion distillation (issue #94) extends this module with its own
//! `DistilledConclusion*` schemas and a `distill_conclusions` fn; this slice
//! ships only the Activity path.

use std::collections::HashMap;

use capture_types::ActivityCategory;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use app_infra::{CaptureWindow, NewActivity, NewActivityEvidence, UserContextStore};

/// System instruction for the Activity-segmentation pass. Kept terse: the
/// detailed item formatting + the return shape live in the per-call prompt and
/// the `DerivedActivityBatch` JSON schema.
const ACTIVITY_PREAMBLE: &str = "You analyze a chronological stretch of a single user's captured \
on-screen text and spoken transcripts and segment it into semantic Activity episodes. An \
Activity is a coherent unit of work or intent — its boundaries are INTENT SHIFTS (for example \
\"stopped wrestling the deploy, started writing the design doc\"), NOT app switches or fixed time \
windows. A single Activity may span multiple apps, and a single app may host several Activities. \
Do not emit one Activity per app or per time slice. For each Activity give a short title, a one \
or two sentence summary of what the user was doing and how, an optional category from this fixed \
taxonomy (coding, research, communication, design, testing, personal, distractions) or omit it \
when unsure, and the list of evidence reference tags (the f<id>/a<id> tags shown on each input \
item) that belong to that Activity. Only use tags that appear in the input. Return the structured \
result.";

/// Per-item text cap so a single noisy capture cannot dominate the prompt budget.
const ITEM_TEXT_CHAR_CAP: usize = 1200;

/// One Activity episode as returned by the engine. `evidence_refs` are the
/// `f<id>`/`a<id>` tags (frame / audio_segment) that ground the episode.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DerivedActivity {
    pub title: String,
    pub summary: String,
    /// Optional category; snake_case from the fixed taxonomy. Unknown → dropped.
    #[serde(default)]
    pub category: Option<String>,
    /// `f<id>` (frame) / `a<id>` (audio_segment) evidence tags.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

/// The structured batch the engine returns for one capture window.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DerivedActivityBatch {
    pub activities: Vec<DerivedActivity>,
}

/// Best-effort token estimate (≈4 chars/token). rig-core's extractor does not
/// surface exact provider usage, so the derivation-run ledger records this
/// approximation rather than a billed count.
pub fn estimate_tokens(text: &str) -> i64 {
    (text.chars().count() as i64 + 3) / 4
}

/// The outcome of one [`derive_activities`] call. The caller stamps a single
/// `derivation_run` ledger row from this (count + estimated tokens) in one shot,
/// which is why `derive_activities` does not write the run itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityDerivationOutcome {
    /// Number of Activities inserted into the store.
    pub inserted: usize,
    /// Best-effort estimated input tokens (preamble + prompt).
    pub input_tokens: i64,
    /// Best-effort estimated output tokens (serialized extracted batch).
    pub output_tokens: i64,
}

/// Tag a window item as `f<id>` (frame) or `a<id>` (audio_segment). Unknown
/// subject types fall back to the raw type prefix so refs stay unique.
fn item_tag(subject_type: &str, subject_id: i64) -> String {
    match subject_type {
        "frame" => format!("f{subject_id}"),
        "audio_segment" => format!("a{subject_id}"),
        other => format!("{other}{subject_id}"),
    }
}

/// Parse an `f<id>` / `a<id>` evidence ref back to `(subject_type, subject_id)`.
/// Returns `None` for refs that are not a known prefix + integer.
fn parse_ref(reference: &str) -> Option<(&'static str, i64)> {
    let reference = reference.trim();
    if let Some(rest) = reference.strip_prefix('f') {
        rest.parse::<i64>().ok().map(|id| ("frame", id))
    } else if let Some(rest) = reference.strip_prefix('a') {
        rest.parse::<i64>().ok().map(|id| ("audio_segment", id))
    } else {
        None
    }
}

/// Map the engine's snake_case category string onto [`ActivityCategory`].
/// Unknown / empty → `None` (the Activity is still stored, just uncategorized).
fn parse_category(raw: &Option<String>) -> Option<ActivityCategory> {
    let raw = raw.as_deref()?.trim().to_ascii_lowercase();
    match raw.as_str() {
        "coding" => Some(ActivityCategory::Coding),
        "research" => Some(ActivityCategory::Research),
        "communication" => Some(ActivityCategory::Communication),
        "design" => Some(ActivityCategory::Design),
        "testing" => Some(ActivityCategory::Testing),
        "personal" => Some(ActivityCategory::Personal),
        "distractions" => Some(ActivityCategory::Distractions),
        _ => None,
    }
}

/// Truncate on a char boundary to at most `cap` characters.
fn truncate_chars(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cap).collect();
    out.push('…');
    out
}

/// Render the capture window into the per-call prompt. Each item is tagged
/// `f<id>`/`a<id>` with its time, optional Search Context app/url, and its
/// (truncated, already-redacted) text.
fn build_prompt(window: &CaptureWindow) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "Below is a chronological list of capture items from one window of the user's activity. \
Each item is tagged with an id (f<id> = on-screen text frame, a<id> = audio transcript segment), \
its capture time in unix milliseconds, and (when known) the app/URL it came from. Segment these \
items into Activity episodes by intent shift and return DerivedActivityBatch.\n\n",
    );
    prompt.push_str(&format!(
        "Window: [{} .. {}] ms ({} items)\n\n",
        window.start_ms,
        window.end_ms,
        window.items.len()
    ));

    for item in &window.items {
        let tag = item_tag(&item.subject_type, item.subject_id);
        prompt.push_str(&format!("[{tag}] t={}ms", item.captured_at_ms));
        if let Some(app) = item.app_label.as_deref().filter(|s| !s.trim().is_empty()) {
            prompt.push_str(&format!(" app={app}"));
        }
        if let Some(url) = item.url.as_deref().filter(|s| !s.trim().is_empty()) {
            prompt.push_str(&format!(" url={url}"));
        }
        prompt.push('\n');
        let text = truncate_chars(item.text.trim(), ITEM_TEXT_CHAR_CAP);
        prompt.push_str(&text);
        prompt.push_str("\n\n");
    }

    prompt
}

/// Derive **Activity** episodes from one redacted capture window and persist
/// them. Returns the count inserted plus best-effort token estimates.
///
/// The caller (the worker / the run-now command) owns recording the single
/// `derivation_run` ledger row from the returned [`ActivityDerivationOutcome`],
/// which is why this fn does not write the run itself.
///
/// `provider_label` / `model_label` are accepted only so the signature matches
/// the caller's run-row stamping; they are not sent to the engine (the engine
/// selection lives in `engine`).
pub async fn derive_activities(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    window: CaptureWindow,
    _provider_label: Option<String>,
    _model_label: Option<String>,
) -> Result<ActivityDerivationOutcome, String> {
    if window.items.is_empty() {
        return Ok(ActivityDerivationOutcome {
            inserted: 0,
            input_tokens: 0,
            output_tokens: 0,
        });
    }

    // Index each window item's captured_at by its tag so derived evidence refs
    // can be resolved back to a capture time without re-querying.
    let mut tag_to_captured_at: HashMap<String, i64> = HashMap::new();
    for item in &window.items {
        tag_to_captured_at.insert(
            item_tag(&item.subject_type, item.subject_id),
            item.captured_at_ms,
        );
    }

    let prompt = build_prompt(&window);
    let input_tokens = estimate_tokens(ACTIVITY_PREAMBLE) + estimate_tokens(&prompt);

    let batch: DerivedActivityBatch =
        ai_engine::extract_with_preamble::<DerivedActivityBatch>(engine, ACTIVITY_PREAMBLE, &prompt)
            .await
            .map_err(|error| error.to_string())?;

    let output_tokens = serde_json::to_string(&batch)
        .map(|json| estimate_tokens(&json))
        .unwrap_or(0);

    let mut inserted = 0usize;
    for activity in &batch.activities {
        // Resolve evidence refs that are actually present in the window, dedup,
        // and pull each capture time.
        let mut evidence: Vec<NewActivityEvidence> = Vec::new();
        let mut seen: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
        let mut captured_at_values: Vec<i64> = Vec::new();
        for reference in &activity.evidence_refs {
            let Some((subject_type, subject_id)) = parse_ref(reference) else {
                continue;
            };
            let tag = item_tag(subject_type, subject_id);
            let Some(&captured_at) = tag_to_captured_at.get(&tag) else {
                // A ref the engine invented (not in this window) is ignored.
                continue;
            };
            if !seen.insert((subject_type.to_string(), subject_id)) {
                continue;
            }
            captured_at_values.push(captured_at);
            evidence.push(NewActivityEvidence {
                subject_type: subject_type.to_string(),
                subject_id,
                captured_at_ms: Some(captured_at),
            });
        }

        // No resolvable evidence → ungrounded; skip (never store a free-floating
        // Activity).
        if evidence.is_empty() {
            continue;
        }

        let started_at_ms = captured_at_values
            .iter()
            .copied()
            .min()
            .unwrap_or(window.start_ms);
        let ended_at_ms = captured_at_values
            .iter()
            .copied()
            .max()
            .unwrap_or(window.end_ms);

        let title = activity.title.trim();
        let title = if title.is_empty() { "Activity" } else { title };

        let draft = NewActivity {
            title: title.to_string(),
            summary: activity.summary.trim().to_string(),
            category: parse_category(&activity.category),
            started_at_ms,
            ended_at_ms,
            derivation_run_id: None,
            evidence,
        };

        store
            .insert_activity_with_evidence(draft)
            .await
            .map_err(|error| error.to_string())?;
        inserted += 1;
    }

    Ok(ActivityDerivationOutcome {
        inserted,
        input_tokens,
        output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_and_audio_refs() {
        assert_eq!(parse_ref("f12"), Some(("frame", 12)));
        assert_eq!(parse_ref(" a3 "), Some(("audio_segment", 3)));
        assert_eq!(parse_ref("x9"), None);
        assert_eq!(parse_ref("frame"), None);
    }

    #[test]
    fn parses_known_categories_only() {
        assert_eq!(
            parse_category(&Some("Coding".to_string())),
            Some(ActivityCategory::Coding)
        );
        assert_eq!(
            parse_category(&Some("distractions".to_string())),
            Some(ActivityCategory::Distractions)
        );
        assert_eq!(parse_category(&Some("unknown".to_string())), None);
        assert_eq!(parse_category(&None), None);
    }

    #[test]
    fn truncates_long_text_on_char_boundary() {
        let text = "a".repeat(ITEM_TEXT_CHAR_CAP + 50);
        let truncated = truncate_chars(&text, ITEM_TEXT_CHAR_CAP);
        // cap chars + the ellipsis.
        assert_eq!(truncated.chars().count(), ITEM_TEXT_CHAR_CAP + 1);
    }
}
