//! LLM orchestration for **Activity** derivation (issue #93).
//!
//! Builds a prompt from a redacted [`app_infra::CaptureWindow`], asks the
//! configured [`ai_engine::EngineConfig`] to segment the window into semantic
//! **Activity** episodes (bounded by *intent shifts*, not per-app or per-time
//! slices), and persists each one with its raw-capture evidence via
//! [`app_infra::UserContextStore::insert_activity_with_evidence`].
//!
//! Conclusion distillation (issue #94) lives in the second half of this module:
//! the `DistilledConclusion*` schemas and `distill_conclusions`, which read
//! accumulated **Activity** episodes and ask the engine to form open-ended,
//! plain-language **Conclusion** statements grounded in Activity evidence.

use std::collections::HashMap;

use capture_types::{
    ActivityCategory, AuthoredContext, DismissalState, EvidenceStance, FocusLevel,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use app_infra::{
    evidence_fingerprint, ActivityCorrection, CaptureWindow, NewActivity, NewActivityEvidence,
    NewConclusion, NewConclusionEvidence, SubjectVectorStore, SupersedeOutcome, UserContextStore,
};
use app_infra::user_context::{confidence, guardrail};

/// System instruction for the Activity-segmentation pass. Kept terse: the
/// detailed item formatting + the return shape live in the per-call prompt and
/// the `DerivedActivityBatch` JSON schema.
const ACTIVITY_PREAMBLE: &str = "You analyze a chronological stretch of a single user's captured \
on-screen text and spoken transcripts and segment it into semantic Activity episodes. An \
Activity is a coherent unit of work or intent — its boundaries are INTENT SHIFTS (for example \
\"stopped wrestling the deploy, started writing the design doc\"), NOT app switches or fixed time \
windows. A single Activity may span multiple apps, and a single app may host several Activities. \
Do not emit one Activity per app or per time slice. For each Activity give a short title, a one \
or two sentence summary of what the user was doing and how, and an optional category from this \
fixed taxonomy or omit it when unsure: creating (producing anything — code, documents, designs, \
slides, music), communication (asynchronous text — email, chat, messages), meetings (real-time \
conversation — calls, video meetings), research (reading or searching in service of the current \
task), learning (deliberate skill-building for its own sake — courses, tutorials), organizing \
(structuring work or time — calendar, task managers, planning, admin paperwork), personal (life \
errands regardless of subject — shopping, banking, health, travel), entertainment (videos, games, \
social feeds, browsing for fun). Category boundaries: synchronous conversation is meetings while \
asynchronous text is communication; reading in service of the current task is research while \
deliberate skill-building for its own sake is learning; life errands are personal even when \
work-adjacent while structuring work or time is organizing. Also give an optional focus level \
from this fixed taxonomy (deep = sustained single-thread deep \
work, mixed = some focus but context-switching or interleaved, distracted = scattered, interrupted, \
or off-task) or omit it when unsure, and the list of evidence reference tags (the f<id>/a<id> tags \
shown on each input item) that belong to that Activity. Only use tags that appear in the input. \
Also give a headline_ref: the SINGLE evidence tag from that list that best represents the title — \
the one moment someone should see first to recognize the Activity (must be one of the Activity's \
own evidence tags). Do NOT describe an \
Activity, or label its category or focus, in terms of the user's health or mental health, sexual \
orientation, religion, political views, or similar protected/intimate domains; keep titles and \
summaries to the work/task itself. Return the structured result.";

/// Per-item text cap so a single noisy capture cannot dominate the prompt budget.
const ITEM_TEXT_CHAR_CAP: usize = 1200;

/// How many of the user's most-recent Category/Focus corrections (#108) are fed
/// back into the Activity-derivation prompt. Newest-first; older corrections are
/// dropped. Bounds the prompt growth from a heavy corrector.
const CORRECTION_FEEDBACK_LIMIT: i64 = 30;

/// Total char budget for the USER CORRECTIONS feedback block (#108). Corrections
/// are included newest-first until the next would exceed this cap.
const CORRECTION_FEEDBACK_CHAR_CAP: usize = 2_000;

/// Per-correction title/summary cap inside the feedback block, so one verbose
/// Activity cannot dominate the corrections budget.
const CORRECTION_ITEM_CHAR_CAP: usize = 160;

/// One Activity episode as returned by the engine. `evidence_refs` are the
/// `f<id>`/`a<id>` tags (frame / audio_segment) that ground the episode.
///
/// `#[schemars(inline)]`: the engine schema must stay `$ref`/`$defs`-free.
/// schemars renders a nested struct as a `#/$defs/DerivedActivity` reference by
/// default, but several structured-output backends (vLLM/outlines-style guided
/// decoding behind OpenAI-compatible/local endpoints) cannot resolve schema
/// references and reject the request with `Error resolving schema reference`.
/// Inlining emits the object schema directly inside `DerivedActivityBatch`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(inline)]
pub struct DerivedActivity {
    pub title: String,
    pub summary: String,
    /// Optional category; snake_case from the fixed taxonomy. Unknown → dropped.
    #[serde(default)]
    pub category: Option<String>,
    /// Optional focus level; snake_case from the fixed taxonomy
    /// (deep / mixed / distracted). Unknown → dropped (#105).
    #[serde(default)]
    pub focus: Option<String>,
    /// `f<id>` (frame) / `a<id>` (audio_segment) evidence tags.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// The single evidence tag (from `evidence_refs`) that best represents the
    /// title — the frame the Timeline should land on when the user opens this
    /// Activity. Optional: when omitted or not present in `evidence_refs`, the
    /// store falls back to the chronologically earliest evidence frame.
    #[serde(default)]
    pub headline_ref: Option<String>,
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
/// Legacy v1 spellings stay accepted as aliases (`coding`/`design`/`testing` →
/// `Creating`, `distractions` → `Entertainment`; ADR 0032). Unknown / empty →
/// `None` (the Activity is still stored, just uncategorized).
fn parse_category(raw: &Option<String>) -> Option<ActivityCategory> {
    let raw = raw.as_deref()?.trim().to_ascii_lowercase();
    match raw.as_str() {
        "creating" => Some(ActivityCategory::Creating),
        "communication" => Some(ActivityCategory::Communication),
        "meetings" => Some(ActivityCategory::Meetings),
        "research" => Some(ActivityCategory::Research),
        "learning" => Some(ActivityCategory::Learning),
        "organizing" => Some(ActivityCategory::Organizing),
        "personal" => Some(ActivityCategory::Personal),
        "entertainment" => Some(ActivityCategory::Entertainment),
        "coding" | "design" | "testing" => Some(ActivityCategory::Creating),
        "distractions" => Some(ActivityCategory::Entertainment),
        _ => None,
    }
}

/// Map the engine's snake_case focus string onto [`FocusLevel`] (#105).
/// Unknown / empty → `None` (the Activity is still stored, just unfocused-label).
fn parse_focus(raw: &Option<String>) -> Option<FocusLevel> {
    let raw = raw.as_deref()?.trim().to_ascii_lowercase();
    match raw.as_str() {
        "deep" => Some(FocusLevel::Deep),
        "mixed" => Some(FocusLevel::Mixed),
        "distracted" => Some(FocusLevel::Distracted),
        _ => None,
    }
}

/// Truncate on a char boundary to at most `cap` characters. Shared with the
/// Digest prompt builder (`super::digest`).
pub(crate) fn truncate_chars(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut out: String = text.chars().take(cap).collect();
    out.push('…');
    out
}

/// Format a unix-millis instant as a compact UTC wall-clock string
/// (`2026-06-11 16:42 UTC`). The capture store is all UTC, so every time the
/// model sees here is UTC; labeling it explicitly keeps the model from guessing
/// a timezone. Done by hand to avoid pulling in a format-description.
fn format_utc_ms(ms: i64) -> String {
    let dt = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let clock = dt.time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        date.year(),
        u8::from(date.month()),
        date.day(),
        clock.hour(),
        clock.minute(),
    )
}

/// Render the capture window into the per-call prompt. Each item is tagged
/// `f<id>`/`a<id>` with its time, optional Search Context app/url, and its
/// (truncated, already-redacted) text.
fn build_prompt(window: &CaptureWindow) -> String {
    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    let mut prompt = String::new();
    prompt.push_str(
        "Below is a chronological list of capture items from one window of the user's activity. \
Each item is tagged with an id (f<id> = on-screen text frame, a<id> = audio transcript segment), \
its capture time, and (when known) the app/URL it came from. All times are UTC. Segment these \
items into Activity episodes by intent shift and return DerivedActivityBatch.\n\n",
    );
    prompt.push_str(&format!("Current time: {}\n", format_utc_ms(now_ms)));
    prompt.push_str(&format!(
        "Window (UTC): [{} .. {}] ({} items)\n\n",
        format_utc_ms(window.start_ms),
        format_utc_ms(window.end_ms),
        window.items.len()
    ));

    for item in &window.items {
        let tag = item_tag(&item.subject_type, item.subject_id);
        prompt.push_str(&format!("[{tag}] t={}", format_utc_ms(item.captured_at_ms)));
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

/// snake_case wire label for a corrected Category/Focus, or `"unset"` when the
/// user corrected the label to "none". Used by the corrections feedback block.
fn category_correction_label(category: Option<ActivityCategory>) -> &'static str {
    category.map(category_label).unwrap_or("unset")
}

fn focus_correction_label(focus: Option<FocusLevel>) -> &'static str {
    match focus {
        Some(FocusLevel::Deep) => "deep",
        Some(FocusLevel::Mixed) => "mixed",
        Some(FocusLevel::Distracted) => "distracted",
        None => "unset",
    }
}

/// Render the USER CORRECTIONS prompt block (#108): the user's past
/// Category/Focus corrections, fed back to the engine as a soft "respect these"
/// instruction so it does not regenerate a label the user already corrected away
/// on a similar Activity. Newest-first, kept until [`CORRECTION_FEEDBACK_CHAR_CAP`]
/// is reached (older corrections dropped). Empty (no trailing block) when the
/// user has corrected nothing, so a default prompt is unchanged. This is soft
/// guidance only; the hard guarantee is that a stored correction always wins on
/// read (the store coalesces it over any fresh engine label).
fn build_corrections_block(corrections: &[ActivityCorrection]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for correction in corrections {
        let title = truncate_chars(correction.title.trim(), CORRECTION_ITEM_CHAR_CAP);
        let summary = truncate_chars(correction.summary.trim(), CORRECTION_ITEM_CHAR_CAP);
        let line = format!(
            "- category={} focus={} activity=\"{title}\" — {summary}\n",
            category_correction_label(correction.corrected_category),
            focus_correction_label(correction.corrected_focus),
        );
        if used + line.chars().count() > CORRECTION_FEEDBACK_CHAR_CAP && !lines.is_empty() {
            break;
        }
        used += line.chars().count();
        lines.push(line);
    }
    if lines.is_empty() {
        return String::new();
    }

    let mut block = String::new();
    block.push_str(
        "\nUSER CORRECTIONS — for each Activity below the user CORRECTED the category and/or focus \
you (or a prior pass) assigned to what is shown here. These are authoritative: when you encounter \
a similar Activity, label its category and focus the way the user corrected it, and never \
re-assign the label they corrected away. \"unset\" means the user deliberately removed that \
label.\n\n",
    );
    for line in lines {
        block.push_str(&line);
    }
    block
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

    // Correction feedback loop (#108): the user's past Category/Focus
    // corrections, newest first. Appended to the prompt as a soft "respect these"
    // block so the engine is biased away from regenerating a corrected-away label
    // on a similar Activity. The hard backstop is that a stored correction always
    // WINS on read (the store coalesces it over any fresh engine label), so even
    // if the engine ignores this the corrected Activity keeps the user's label.
    let corrections = store
        .list_activity_corrections(CORRECTION_FEEDBACK_LIMIT)
        .await
        .map_err(|error| error.to_string())?;

    let mut prompt = build_prompt(&window);
    prompt.push_str(&build_corrections_block(&corrections));
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
        // The engine-nominated headline tag, kept only if it resolves to one of
        // this Activity's own evidence tags (else the store falls back to the
        // earliest frame).
        let headline_tag = activity
            .headline_ref
            .as_deref()
            .and_then(parse_ref)
            .map(|(subject_type, subject_id)| item_tag(subject_type, subject_id));

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
                is_headline: headline_tag.as_deref() == Some(tag.as_str()),
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
            focus: parse_focus(&activity.focus),
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

// === #94: Conclusion distillation =========================================

/// System instruction for the Conclusion-distillation pass. Describes the task:
/// form open-ended, plain-language beliefs about the user grounded in the listed
/// Activities, each carrying a Subject and the Activity ids that are its
/// supporting (and any contradicting) evidence.
///
/// The **soft** half of the Sensitive Category Guardrail (#96) prepends
/// [`guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION`] to this base text via
/// [`conclusion_preamble`] before it reaches the engine; the **hard**
/// `guardrail::is_sensitive` post-filter is hooked at the per-conclusion persist
/// site below, since the soft instruction alone is not enough (ADR 0030).
const CONCLUSION_PREAMBLE_BASE: &str = "You read a list of a single user's recent Activity episodes \
(each with an id, a title, a one or two sentence summary, a capture time, and an optional category) \
and distill open-ended, plain-language Conclusion statements about the user. A Conclusion is a \
natural-language belief such as \"Has been increasingly interested in Apple\" or \"Prefers async \
communication\" — NOT a fixed subject+attribute+value row and NOT a tag. Each Conclusion is ABOUT a \
Subject: a short grouping handle like \"Apple\" or \"async communication\". Ground every Conclusion \
in evidence: list the Activity ids that SUPPORT it, and (only when an Activity genuinely cuts \
against it) the Activity ids that CONTRADICT it. Only reference Activity ids that appear in the \
input. Prefer a few well-supported Conclusions over many flimsy ones. When a KNOWN SUBJECTS list \
is provided and a Conclusion is about one of those handles, reuse that handle VERBATIM as its \
subject so it reinforces the existing subject; only invent a new handle for a genuinely new \
subject. Split a compound statement into one Conclusion per DISTINCT claim that can independently \
gain evidence / be confirmed / dismissed / fade — NOT one per proper noun. For example, \"Plays \
Genshin Impact via a Windows VM and watches Marvel Rivals / OW2 streams\" becomes THREE Conclusions \
under subject \"Gaming\": one that they play Genshin Impact (on a Windows VM), one that they \
play/are interested in 007 First Light, and one that they watch gaming streams (Marvel Rivals / \
OW2) — each carrying only its own supporting Activity ids. When a KNOWN SUBJECTS entry lists an \
existing conclusion (shown as `id: statement`) that restates the belief you are forming, set that \
belief's `reinforces_id` to that conclusion's id so it reinforces the existing belief. Otherwise \
leave `reinforces_id` unset (a genuinely new belief). If an existing conclusion shown in KNOWN \
SUBJECTS is now WRONG in light of the evidence and this belief replaces it, set that belief's \
`supersedes_id` to the wrong conclusion's id. `supersedes_id` composes with `reinforces_id` (you \
may reinforce the correct sibling and supersede the wrong one in the same belief). Cite it ONLY \
for a genuinely wrong existing belief, never for a merely weaker or older one. Return the \
structured result.";

/// The full Conclusion-distillation preamble the engine sees: the **soft**
/// Sensitive Category Guardrail instruction (#96) prepended to
/// [`CONCLUSION_PREAMBLE_BASE`]. The guardrail leads so the off-limits-category
/// rule frames the whole task before the engine reads what to produce. The
/// engine never sees the bare base text.
fn conclusion_preamble() -> String {
    format!(
        "{}\n\n{}",
        guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION,
        CONCLUSION_PREAMBLE_BASE
    )
}

/// Number of recent Activities pulled into one distillation pass.
const DISTILLATION_ACTIVITY_LIMIT: i64 = 60;

/// Per-summary text cap so one verbose Activity summary cannot dominate the
/// prompt budget.
const ACTIVITY_SUMMARY_CHAR_CAP: usize = 600;

/// One distilled Conclusion as returned by the engine. `support_refs` /
/// `contradict_refs` are Activity ids (matched back against the pulled set).
///
/// `#[schemars(inline)]`: see [`DerivedActivity`] — keep the batch schema free
/// of `$ref`/`$defs` so reference-less structured-output backends accept it.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(inline)]
pub struct DistilledConclusion {
    /// The Subject the Conclusion is about (a short grouping handle).
    pub subject: String,
    /// The open-ended, plain-language belief statement.
    pub statement: String,
    /// Activity ids that support the Conclusion.
    #[serde(default)]
    pub support_refs: Vec<i64>,
    /// Activity ids that contradict the Conclusion (usually empty).
    #[serde(default)]
    pub contradict_refs: Vec<i64>,
    /// The `id` of an existing conclusion (from a KNOWN SUBJECTS entry) this belief
    /// reinforces, when it restates one already listed; unset (None) for a new belief.
    #[serde(default)]
    pub reinforces_id: Option<i64>,
    /// The `id` of an existing conclusion (from a KNOWN SUBJECTS entry) this belief
    /// SUPERSEDES — a genuinely WRONG existing belief this one replaces (ADR 0046).
    /// Composes with `reinforces_id`; unset (None) unless replacing a wrong belief.
    #[serde(default)]
    pub supersedes_id: Option<i64>,
}

/// The structured batch the engine returns for one distillation pass.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DistilledConclusionBatch {
    pub conclusions: Vec<DistilledConclusion>,
}

/// The outcome of one [`distill_conclusions`] call. The caller stamps a single
/// `derivation_run` ledger row (kind `'conclusion'`) from this, mirroring the
/// [`ActivityDerivationOutcome`] single-shot pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConclusionDistillationOutcome {
    /// Number of Conclusions inserted/updated in the store.
    pub upserted: usize,
    /// How many engine drafts each deterministic persist gate withheld, in
    /// gate order. Recorded on the ledger row so "distillation produced
    /// nothing" is diagnosable (empty engine output vs policy drops).
    pub gate_drops: app_infra::DistillationGateDrops,
    /// Best-effort estimated input tokens (preamble + prompt).
    pub input_tokens: i64,
    /// Best-effort estimated output tokens (serialized extracted batch).
    pub output_tokens: i64,
}

/// Render the distillation prompt: one line per Activity (id, time, category,
/// title) plus its truncated summary.
fn build_distillation_prompt(activities: &[capture_types::Activity]) -> String {
    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    let mut prompt = String::new();
    prompt.push_str(
        "Below is a list of the user's recent Activity episodes, newest first. Each is tagged with \
its numeric Activity id and its start time. All times are UTC. Distill open-ended Conclusion \
statements about the user and reference the Activity ids that are each Conclusion's supporting (and \
any contradicting) evidence. Return DistilledConclusionBatch.\n\n",
    );
    prompt.push_str(&format!("Current time: {}\n", format_utc_ms(now_ms)));
    prompt.push_str(&format!("Activities ({}):\n\n", activities.len()));

    for activity in activities {
        let category = activity
            .category
            .map(category_label)
            .unwrap_or("uncategorized");
        prompt.push_str(&format!(
            "[id={}] t={} category={category} title={}\n",
            activity.id,
            format_utc_ms(activity.started_at_ms),
            activity.title.trim()
        ));
        let summary = truncate_chars(activity.summary.trim(), ACTIVITY_SUMMARY_CHAR_CAP);
        if !summary.is_empty() {
            prompt.push_str(&summary);
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    prompt
}

/// snake_case label for an [`ActivityCategory`] (matches the capture-types serde
/// rename) used in the distillation prompt and the Digest prompt
/// (`super::digest`).
pub(crate) fn category_label(category: ActivityCategory) -> &'static str {
    match category {
        ActivityCategory::Creating => "creating",
        ActivityCategory::Communication => "communication",
        ActivityCategory::Meetings => "meetings",
        ActivityCategory::Research => "research",
        ActivityCategory::Learning => "learning",
        ActivityCategory::Organizing => "organizing",
        ActivityCategory::Personal => "personal",
        ActivityCategory::Entertainment => "entertainment",
    }
}

/// Distill **Conclusion** statements from the accumulated **Activity** episodes
/// and persist each one grounded in its Activity evidence. Returns the count
/// upserted plus best-effort token estimates.
///
/// Mirrors [`derive_activities`]'s single-shot shape: the caller (the worker /
/// the run-now command) owns recording the single `derivation_run` ledger row
/// (kind `'conclusion'`) from the returned [`ConclusionDistillationOutcome`].
///
/// Distillation is a no-op below two Activities: intent / belief signal needs at
/// least a couple of episodes to form a non-flimsy Conclusion.
pub async fn distill_conclusions(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    app_handle: &tauri::AppHandle,
    subject_vectors: &SubjectVectorStore,
) -> Result<ConclusionDistillationOutcome, String> {
    let activities = store
        .activities_for_distillation(DISTILLATION_ACTIVITY_LIMIT)
        .await
        .map_err(|error| error.to_string())?;

    // Nothing to distill from a single (or zero) Activity.
    if activities.len() < 2 {
        return Ok(ConclusionDistillationOutcome {
            upserted: 0,
            gate_drops: Default::default(),
            input_tokens: 0,
            output_tokens: 0,
        });
    }

    // Dismissal State (#99): every Conclusion the user has rejected, with which
    // evidence and when. This feeds BOTH the prompt (a soft "do not reconstitute"
    // instruction) and the deterministic resurface gate at the persist site below.
    let dismissals = store
        .list_dismissals()
        .await
        .map_err(|error| error.to_string())?;

    // User-authored Context (#107): standing statements the user wrote about
    // themselves ("I'm a designer", "I care about X"). These are user-ASSERTED
    // (not derived), so they steer which Conclusions form and what Subjects matter
    // — fed to the engine as authoritative standing context, distinct from the
    // derived Activity evidence. They do NOT bypass the guardrail/formation-bar/
    // resurface gates applied to the engine's output below.
    let authored = store
        .list_authored_context()
        .await
        .map_err(|error| error.to_string())?;

    let valid_ids: std::collections::HashSet<i64> = activities.iter().map(|a| a.id).collect();
    let started_at_by_id: HashMap<i64, i64> = activities
        .iter()
        .map(|a| (a.id, a.started_at_ms))
        .collect();

    // Soft guardrail (#96): the engine sees the Sensitive Category Guardrail
    // instruction prepended to the base preamble — it must not form conclusions
    // about health, sexuality, religion, politics, or similar intimate domains.
    let preamble = conclusion_preamble();
    let mut prompt = build_distillation_prompt(&activities);
    // Prepend the USER-AUTHORED CONTEXT block (#107): standing statements the user
    // wrote about themselves, fed as authoritative steering context (distinct from
    // derived Activity evidence). Appended context only — it does not touch the
    // Activity list above. Empty (unchanged prompt) when the user has authored none.
    prompt.push_str(&build_authored_context_block(&authored));
    // Append the DISMISSED-CONCLUSIONS block (#99): tell the engine not to simply
    // reconstitute corrections the user already made unless substantially MORE and
    // NEWER evidence rebuilds them. This is appended context only — it does not
    // touch the seedQuery / window text. The deterministic resurface gate at the
    // persist site is the hard backstop for when the engine ignores this.
    prompt.push_str(&build_dismissed_conclusions_block(&dismissals));
    // KNOWN SUBJECTS block (slices 5/6): feed the engine the subject handles the
    // user already has so it reuses one VERBATIM (which then reinforces the
    // canonical Subject row via the subject-only upsert) instead of coining a
    // reworded duplicate. The candidate set is the UNION of three sources, with the
    // LLM as the matcher over all of them:
    //   * a recency floor — the newest distinct handles, always included;
    //   * the LEXICAL leg — existing Subjects whose name/statements share words with
    //     the recent Activity text (model-free, no embedding-backfill lag); and
    //   * Mode 1 (semantic) — when a Semantic Search model is installed, the
    //     window's Activities embedded and KNN'd against the stored Subject Vectors
    //     for non-lexically-related handles (e.g. "Apple" ↔ "iPhone").
    // The recency floor + lexical leg are load-bearing: a Subject created by a recent
    // distillation is not embedded into the Subject Vectors until the backfill worker
    // runs *after* it, so semantic KNN structurally cannot surface the freshest
    // Subjects — exactly the ones the next distillation re-derives and duplicates.
    // The lexical leg catches the common case (a reworded duplicate shares words) with
    // no model at all, so it works in the default/prod config too. Unioning (not the
    // old `semantic OR recency` either/or, whose non-empty semantic set suppressed the
    // fallback and let a fresh Subject get reworded) closes the gap. Appended steering
    // context (like the authored / dismissed blocks); it does not touch the Activity
    // list above.
    let activity_query = activities
        .iter()
        .map(|activity| format!("{} {}", activity.title.trim(), activity.summary.trim()))
        .collect::<Vec<_>>()
        .join(" ");
    let lexical_handles = store
        .list_subject_handles_by_lexical_overlap(&activity_query, KNOWN_SUBJECTS_LEXICAL_LIMIT)
        .await
        .map_err(|error| error.to_string())?;
    let semantic_candidates =
        super::subject_candidates::select_semantic_subject_candidates(
            app_handle,
            subject_vectors,
            &activities,
        )
        .await;
    let recency_handles = store
        .list_subject_handles_by_recency(KNOWN_SUBJECTS_FALLBACK_LIMIT)
        .await
        .map_err(|error| error.to_string())?;
    // Related = lexical first (most precise for dedup), then semantic. merge_known_subjects
    // leads with the recency floor, then these related handles (so an OLD lexical/semantic
    // duplicate survives the char cap ahead of the older recency tail), then the rest of
    // recency. Dedup is case-insensitive across all three.
    let mut related = lexical_handles;
    related.extend(semantic_candidates);
    let known_subjects =
        super::subject_candidates::merge_known_subjects(recency_handles, related);
    // Per-belief reinforce (ADR 0043): show each candidate subject's existing
    // conclusions as `id: statement` lines so the engine can cite the exact belief it
    // reinforces via `reinforces_id`. Tuples arrive confidence-desc within a subject.
    let subject_conclusions = store
        .list_conclusions_for_subjects(&known_subjects, KNOWN_SUBJECTS_CONCLUSIONS_PER_SUBJECT_CAP)
        .await
        .map_err(|error| error.to_string())?;
    let mut conclusions_by_subject: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for (subject, id, statement, _confidence) in &subject_conclusions {
        conclusions_by_subject
            .entry(subject.clone())
            .or_default()
            .push((*id, statement.clone()));
    }
    // The ids actually shown to the model — a `reinforces_id` naming anything else was
    // hallucinated and is dropped to None at the persist site below.
    let shown_ids: std::collections::HashSet<i64> =
        subject_conclusions.iter().map(|(_, id, _, _)| *id).collect();
    prompt.push_str(&build_known_subjects_block(
        &known_subjects,
        &conclusions_by_subject,
    ));
    let input_tokens = estimate_tokens(&preamble) + estimate_tokens(&prompt);

    let batch: DistilledConclusionBatch = ai_engine::extract_with_preamble::<
        DistilledConclusionBatch,
    >(engine, &preamble, &prompt)
    .await
    .map_err(|error| error.to_string())?;

    let output_tokens = serde_json::to_string(&batch)
        .map(|json| estimate_tokens(&json))
        .unwrap_or(0);

    let now = now_ms();
    let mut upserted = 0usize;
    let mut gate_drops = app_infra::DistillationGateDrops::default();
    for conclusion in &batch.conclusions {
        let subject = conclusion.subject.trim();
        let statement = conclusion.statement.trim();
        if subject.is_empty() || statement.is_empty() {
            continue;
        }

        // Keep only refs that name an Activity actually in the pulled set; dedup.
        let support_refs = filter_known_refs(&conclusion.support_refs, &valid_ids);
        let contradict_refs = filter_known_refs(&conclusion.contradict_refs, &valid_ids);

        // No resolvable supporting evidence => ungrounded; skip (never store a
        // free-floating Conclusion).
        if support_refs.is_empty() {
            gate_drops.ungrounded += 1;
            continue;
        }

        // Hard guardrail (#96, ADR 0030): drop any Conclusion that lands in a
        // sensitive inference category (health/mental health, sexual orientation,
        // religion, politics, and similar protected/intimate domains) BEFORE the
        // formation bar and the upsert, so a sensitive Conclusion never enters the
        // store and the `recall_context` broker tool cannot return it. This is the
        // deterministic backstop for when the engine ignores the soft instruction;
        // it deliberately errs toward over-suppression.
        if guardrail::is_sensitive(subject, statement) {
            gate_drops.guardrail_suppressed += 1;
            continue;
        }

        // Formation bar (#95, Confidence Policy): a Conclusion needs at least
        // FORMATION_BAR_EVIDENCE (≥2) supporting Activities before it forms — no
        // flimsy one-afternoon conclusions. Below the bar, skip the upsert.
        if !confidence::meets_formation_bar(support_refs.len()) {
            gate_drops.below_formation_bar += 1;
            continue;
        }

        // Dismissal resurface gate (#99): if the user already dismissed ~this
        // Conclusion (case-insensitive subject AND statement equality), a Dismiss
        // is a reset with a HIGH resurface bar — never a re-form from the same
        // evidence just rejected, and otherwise only on substantially MORE fresh
        // support than it took to form. Ordering: the cheap deterministic drops
        // above (#96 guardrail, #95 formation bar) run first; this is the last gate
        // before the upsert. The fresh fingerprint covers the same distinct
        // evidence set the dismissal recorded (all stances), so an identical
        // evidence set produces an identical fingerprint.
        let mut fresh_evidence: Vec<i64> = support_refs.clone();
        fresh_evidence.extend(contradict_refs.iter().copied());
        let fresh_fingerprint = evidence_fingerprint(&fresh_evidence);
        if let Some(dismissal) = matching_dismissal(&dismissals, subject, statement) {
            if fresh_fingerprint == dismissal.evidence_fingerprint {
                // The exact evidence the user just rejected — never resurface.
                gate_drops.resurface_blocked += 1;
                continue;
            }
            if !confidence::meets_resurface_bar(
                support_refs.len(),
                dismissal.evidence_activity_count,
            ) {
                // Fresh evidence exists but does not clear the high resurface bar;
                // honor the correction (a dismissal must never feel ignored).
                gate_drops.resurface_blocked += 1;
                continue;
            }
            // Resurface bar cleared. ADR 0046: for a SUPERSEDE dismissal (a machine
            // retirement) the old belief was right after all — flip the RETAINED
            // superseded row back to visible with its history rather than forming a
            // duplicate. A user Dismiss (source='user') keeps today's behavior: fall
            // through to form a new row. If no superseded row is actually flipped
            // (e.g. already cleared), fall through to normal formation too.
            if dismissal.source == "supersede" {
                let flipped = store
                    .resurface_superseded(subject, statement)
                    .await
                    .map_err(|error| error.to_string())?;
                if flipped {
                    upserted += 1;
                    continue;
                }
            }
        }

        // started / last_supported = the most recent supporting Activity time
        // (fallback: now if none resolved, though support_refs is non-empty here).
        let last_supported_at_ms = support_refs
            .iter()
            .filter_map(|id| started_at_by_id.get(id).copied())
            .max()
            .unwrap_or(now);

        // Confidence is resolved INSIDE the store's transaction as the
        // reinforcement ratchet (#9/#10): a new Conclusion forms at
        // `initial_confidence(support, contradict)`, an existing one is decayed to
        // now, ratcheted up by fresh support (never reset to a lower window), then
        // dropped by `apply_contradiction` for fresh contradicting evidence. The
        // store needs the resolved support/contradict counts to do this.
        let mut evidence: Vec<NewConclusionEvidence> = Vec::with_capacity(
            support_refs.len() + contradict_refs.len(),
        );
        for id in &support_refs {
            evidence.push(NewConclusionEvidence {
                activity_id: *id,
                stance: EvidenceStance::Support,
            });
        }
        for id in &contradict_refs {
            evidence.push(NewConclusionEvidence {
                activity_id: *id,
                stance: EvidenceStance::Contradict,
            });
        }

        // Per-belief reinforce (ADR 0043): forward the cited id only if we actually
        // showed it to the model — a hallucinated / not-shown id becomes None. The
        // store re-validates subject/dismissed defensively, so this is just "don't
        // forward an id we never presented".
        let reinforces_id = conclusion.reinforces_id.filter(|id| shown_ids.contains(id));
        // Supersede (ADR 0046): forward the cited id only if we actually showed it
        // — same drop-hallucinated rule as reinforces_id. The store enforces
        // same-subject / not-pinned / self-block / retire-only-downward strength.
        let supersedes_id = conclusion.supersedes_id.filter(|id| shown_ids.contains(id));

        // Single transaction: upsert (with the ratcheted confidence) + evidence
        // replacement commit/roll back together, so an error between them can
        // never leave a Conclusion with stale or zero evidence (#14).
        let outcome = store
            .upsert_conclusion_with_evidence(
                NewConclusion {
                    subject: subject.to_string(),
                    statement: statement.to_string(),
                    // Unused by the ratchet path (the store resolves confidence),
                    // kept coherent: the formation value for a brand-new row.
                    confidence: confidence::initial_confidence(
                        support_refs.len(),
                        contradict_refs.len(),
                    ),
                    formed_at_ms: last_supported_at_ms,
                    last_supported_at_ms,
                },
                support_refs.len(),
                contradict_refs.len(),
                evidence,
                reinforces_id,
                supersedes_id,
            )
            .await
            .map_err(|error| error.to_string())?;
        // Count what the optional supersede did (ADR 0046 observability). These are
        // OUTCOME counters, not withholdings — the citing belief still persisted.
        match outcome.supersede {
            SupersedeOutcome::Retired => gate_drops.superseded += 1,
            SupersedeOutcome::Degraded => gate_drops.supersede_degraded += 1,
            SupersedeOutcome::Blocked => gate_drops.supersede_blocked += 1,
            SupersedeOutcome::None => {}
        }
        upserted += 1;
    }

    Ok(ConclusionDistillationOutcome {
        upserted,
        gate_drops,
        input_tokens,
        output_tokens,
    })
}

/// Current unix time in milliseconds (no `Date.now()`-style nondeterminism).
fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Total char budget for the USER-AUTHORED CONTEXT block (#107). Statements are
/// included newest-first (the store lists them that way) until adding the next one
/// would exceed this cap; the remaining (older) statements are dropped. Bounds the
/// prompt growth from a user who authors many statements.
const AUTHORED_CONTEXT_CHAR_CAP: usize = 2_000;

/// Render the USER-AUTHORED CONTEXT prompt block (#107): the user's standing,
/// self-asserted statements, fed to the engine as authoritative context that should
/// steer which Conclusions form and what Subjects matter — clearly labeled as
/// user-asserted, distinct from the derived Activity evidence above. Statements
/// arrive newest-first and are kept until [`AUTHORED_CONTEXT_CHAR_CAP`] is reached
/// (older ones are dropped). Empty (no trailing block) when the user has authored
/// none, so a default dossier's prompt is unchanged.
fn build_authored_context_block(authored: &[AuthoredContext]) -> String {
    // Collect non-empty statement lines newest-first up to the char budget.
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for item in authored {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let line = match item.topic.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(topic) => format!("- (topic: {topic}) {text}\n"),
            None => format!("- {text}\n"),
        };
        if used + line.chars().count() > AUTHORED_CONTEXT_CHAR_CAP && !lines.is_empty() {
            break;
        }
        used += line.chars().count();
        lines.push(line);
    }
    if lines.is_empty() {
        return String::new();
    }

    let mut block = String::new();
    block.push_str(
        "\nUSER-AUTHORED CONTEXT — the user wrote these standing statements about themselves. They \
are ASSERTED by the user (not inferred from the Activities above), so treat them as authoritative \
steering context: let them shape which Conclusions you form and which Subjects matter, and do not \
contradict them. They are not Activity evidence — do not cite them as support_refs.\n\n",
    );
    for line in lines {
        block.push_str(&line);
    }
    block
}

/// Render the DISMISSED-CONCLUSIONS prompt block (#99): the soft half of the
/// resurface rule. Lists each rejected Conclusion's subject + statement and tells
/// the engine these are corrections the user already made — do NOT reconstitute
/// them unless substantially MORE and NEWER evidence than before rebuilds them.
/// Empty (no trailing block) when there are no dismissals, so a fresh dossier's
/// prompt is unchanged.
fn build_dismissed_conclusions_block(dismissals: &[DismissalState]) -> String {
    if dismissals.is_empty() {
        return String::new();
    }
    let mut block = String::new();
    block.push_str(
        "\nDISMISSED CONCLUSIONS — each belief below was already removed: either the user REJECTED \
it as wrong, or a prior pass ALREADY REPLACED it with a correction (marked per row). Do NOT \
reconstitute, restate, or paraphrase any of these unless there is substantially MORE and NEWER \
Activity evidence than before that genuinely rebuilds it; never re-form one from the same evidence \
that was already removed. When in doubt, leave a removed belief out.\n\n",
    );
    for dismissal in dismissals {
        // ADR 0046: supersede rows are machine corrections — present them as
        // "already replaced" rather than the user's "rejected".
        let marker = if dismissal.source == "supersede" {
            "already replaced"
        } else {
            "rejected"
        };
        block.push_str(&format!(
            "- ({marker}) subject={} statement={}\n",
            dismissal.subject.trim(),
            dismissal.statement.trim()
        ));
    }
    block
}

/// How many recency-ordered handles Mode 2 (no Semantic Search model, or an empty
/// semantic pass) pulls from the store as the fallback candidate set. The real
/// bound on what reaches the prompt is [`KNOWN_SUBJECTS_CHAR_CAP`]; this just caps
/// the store read so a user with thousands of Subjects does not over-fetch.
const KNOWN_SUBJECTS_FALLBACK_LIMIT: i64 = 200;

/// How many lexical-overlap candidate handles the model-free leg contributes to the
/// KNOWN SUBJECTS union per distillation. The leg ranks ALL existing Subjects by
/// shared-word relevance to the recent Activity text and keeps this many best; the
/// real bound on the prompt is [`KNOWN_SUBJECTS_CHAR_CAP`]. Kept modest so a wide,
/// topically-scattered window cannot flood the block with weak lexical matches.
const KNOWN_SUBJECTS_LEXICAL_LIMIT: i64 = 20;

/// Total char budget for the KNOWN SUBJECTS block (slices 5/6). Each handle now
/// carries its existing conclusions as indented `id: statement` lines, so the cap
/// budgets those lines too (bumped from 4_000 accordingly). Handles are short but
/// their conclusion lines are not; a big-context cloud LLM effectively sees every
/// candidate, while a small-context local LLM is recency/relevance-bounded by the cap
/// (the accepted worst case — the most-recent/most-relevant handles lead). Calibration-
/// tunable. A subject whose full block (handle + conclusion lines) would overflow
/// degrades to handle-only.
const KNOWN_SUBJECTS_CHAR_CAP: usize = 8_000;

/// Per-subject cap on how many existing conclusions are shown under a KNOWN SUBJECTS
/// handle (confidence-desc). Calibration-tunable, like [`KNOWN_SUBJECTS_CHAR_CAP`].
const KNOWN_SUBJECTS_CONCLUSIONS_PER_SUBJECT_CAP: i64 = 12;

/// Render the KNOWN SUBJECTS prompt block (slices 5/6): the candidate Subject
/// handles the user already has, fed to the engine with an instruction to reuse a
/// handle VERBATIM as a Conclusion's `subject` when the belief is about that
/// subject (so a reworded distillation reinforces the canonical Subject row via the
/// subject-only upsert instead of splitting it into a near-duplicate), and to coin
/// a new handle only for a genuinely new subject. Handles arrive most-relevant /
/// newest-first and are kept until [`KNOWN_SUBJECTS_CHAR_CAP`] is reached (the rest
/// dropped). Empty (no trailing block) when there are no candidate handles, so a
/// fresh dossier's prompt is unchanged — the same convention as the authored /
/// dismissed blocks.
fn build_known_subjects_block(
    handles: &[String],
    conclusions_by_subject: &std::collections::HashMap<String, Vec<(i64, String)>>,
) -> String {
    // Case-insensitive lookup: subjects come from the same store as the handles, but
    // match defensively so a casing drift never silently hides a subject's conclusions.
    let lookup = |handle: &str| -> Option<&Vec<(i64, String)>> {
        conclusions_by_subject
            .iter()
            .find(|(subject, _)| subject.eq_ignore_ascii_case(handle))
            .map(|(_, conclusions)| conclusions)
    };

    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for handle in handles {
        let handle = handle.trim();
        if handle.is_empty() {
            continue;
        }
        let handle_line = format!("- {handle}\n");
        // The full block for this subject = handle line + its indented conclusion lines.
        let conclusion_lines: String = lookup(handle)
            .map(|conclusions| {
                conclusions
                    .iter()
                    .map(|(id, statement)| format!("    {id}: {}\n", statement.trim()))
                    .collect()
            })
            .unwrap_or_default();
        let full = format!("{handle_line}{conclusion_lines}");
        if used + full.chars().count() <= KNOWN_SUBJECTS_CHAR_CAP || lines.is_empty() {
            used += full.chars().count();
            lines.push(full);
            continue;
        }
        // Full block overflows: degrade to handle-only if that alone still fits, else stop.
        if used + handle_line.chars().count() <= KNOWN_SUBJECTS_CHAR_CAP {
            used += handle_line.chars().count();
            lines.push(handle_line);
        }
    }
    if lines.is_empty() {
        return String::new();
    }

    let mut block = String::new();
    block.push_str(
        "\nKNOWN SUBJECTS — these are subject handles the user already has, each followed by its \
existing conclusions as indented `id: statement` lines. When a Conclusion you form is about one of \
these handles, reuse the handle VERBATIM (exactly as written below) as that Conclusion's subject so \
it reinforces the existing subject rather than creating a reworded duplicate. When one of the listed \
`id: statement` conclusions restates the belief you are forming, set that belief's `reinforces_id` \
to that id so it reinforces that exact existing belief. Only coin a NEW handle for a genuinely new \
subject that is not in this list.\n\n",
    );
    for line in lines {
        block.push_str(&line);
    }
    block
}

/// Find the dismissal (if any) that matches a freshly-distilled Conclusion by
/// case-insensitive subject AND case-insensitive statement equality. The match is
/// exact (not fuzzy): a dismissal vetoes the specific belief the user rejected, so
/// the resurface gate keys on the same `(subject, statement)` identity the store
/// dedups Conclusions by.
fn matching_dismissal<'a>(
    dismissals: &'a [DismissalState],
    subject: &str,
    statement: &str,
) -> Option<&'a DismissalState> {
    dismissals.iter().find(|dismissal| {
        dismissal.subject.eq_ignore_ascii_case(subject)
            && dismissal.statement.eq_ignore_ascii_case(statement)
    })
}

/// Filter a list of Activity-id refs to the ones present in `valid_ids`,
/// preserving order and dropping duplicates.
fn filter_known_refs(refs: &[i64], valid_ids: &std::collections::HashSet<i64>) -> Vec<i64> {
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    refs.iter()
        .copied()
        .filter(|id| valid_ids.contains(id))
        .filter(|id| seen.insert(*id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine schema must not contain `$ref`/`$defs`: reference-less
    /// structured-output backends reject `Error resolving schema reference`.
    /// `#[schemars(inline)]` on the nested item types keeps both batch schemas
    /// flat — guard that here so a future field/type change cannot silently
    /// reintroduce a `$ref`.
    #[test]
    fn batch_schemas_are_reference_free() {
        // Walk the JSON structurally: a real reference is an object *key*
        // (`$ref`/`$defs`), not the literal text that appears inside a
        // `description` copied from these types' doc comments.
        fn has_reference(value: &serde_json::Value) -> bool {
            match value {
                serde_json::Value::Object(map) => {
                    map.contains_key("$ref")
                        || map.contains_key("$defs")
                        || map.values().any(has_reference)
                }
                serde_json::Value::Array(items) => items.iter().any(has_reference),
                _ => false,
            }
        }

        for schema in [
            serde_json::to_value(schemars::schema_for!(DerivedActivityBatch)).unwrap(),
            serde_json::to_value(schemars::schema_for!(DistilledConclusionBatch)).unwrap(),
        ] {
            assert!(!has_reference(&schema), "schema has a reference: {schema}");
        }
    }

    #[test]
    fn parses_frame_and_audio_refs() {
        assert_eq!(parse_ref("f12"), Some(("frame", 12)));
        assert_eq!(parse_ref(" a3 "), Some(("audio_segment", 3)));
        assert_eq!(parse_ref("x9"), None);
        assert_eq!(parse_ref("frame"), None);
    }

    #[test]
    fn parses_known_categories_only() {
        for (raw, expected) in [
            ("creating", ActivityCategory::Creating),
            ("communication", ActivityCategory::Communication),
            ("meetings", ActivityCategory::Meetings),
            ("research", ActivityCategory::Research),
            ("learning", ActivityCategory::Learning),
            ("organizing", ActivityCategory::Organizing),
            ("personal", ActivityCategory::Personal),
            ("entertainment", ActivityCategory::Entertainment),
        ] {
            assert_eq!(parse_category(&Some(raw.to_string())), Some(expected), "{raw}");
        }
        assert_eq!(parse_category(&Some("unknown".to_string())), None);
        assert_eq!(parse_category(&Some("  ".to_string())), None);
        assert_eq!(parse_category(&None), None);
    }

    #[test]
    fn parse_category_aliases_legacy_spellings() {
        for (raw, expected) in [
            ("coding", ActivityCategory::Creating),
            ("Coding", ActivityCategory::Creating),
            ("design", ActivityCategory::Creating),
            ("testing", ActivityCategory::Creating),
            ("distractions", ActivityCategory::Entertainment),
        ] {
            assert_eq!(parse_category(&Some(raw.to_string())), Some(expected), "{raw}");
        }
    }

    #[test]
    fn parses_known_focus_levels_only() {
        assert_eq!(parse_focus(&Some("Deep".to_string())), Some(FocusLevel::Deep));
        assert_eq!(parse_focus(&Some(" mixed ".to_string())), Some(FocusLevel::Mixed));
        assert_eq!(
            parse_focus(&Some("distracted".to_string())),
            Some(FocusLevel::Distracted)
        );
        assert_eq!(parse_focus(&Some("unknown".to_string())), None);
        assert_eq!(parse_focus(&None), None);
    }

    fn correction(
        id: i64,
        title: &str,
        category: Option<ActivityCategory>,
        focus: Option<FocusLevel>,
    ) -> ActivityCorrection {
        ActivityCorrection {
            activity_id: id,
            title: title.to_string(),
            summary: format!("{title} summary"),
            corrected_category: category,
            corrected_focus: focus,
            corrected_at_ms: 1_000,
        }
    }

    #[test]
    fn corrections_block_is_empty_without_corrections() {
        assert!(build_corrections_block(&[]).is_empty());
    }

    #[test]
    fn corrections_block_lists_each_correction_with_labels() {
        let corrections = vec![
            correction(
                1,
                "Scrolled social media",
                Some(ActivityCategory::Entertainment),
                Some(FocusLevel::Distracted),
            ),
            // A correction that unset both labels renders as "unset".
            correction(2, "Misc", None, None),
        ];
        let block = build_corrections_block(&corrections);
        assert!(block.contains("USER CORRECTIONS"));
        assert!(block.contains("never re-assign the label they corrected away"));
        assert!(block.contains("category=entertainment focus=distracted"));
        assert!(block.contains("Scrolled social media"));
        assert!(block.contains("category=unset focus=unset"));
    }

    #[test]
    fn corrections_block_respects_char_cap() {
        let long = "x".repeat(200);
        let corrections: Vec<ActivityCorrection> = (0..50)
            .map(|i| {
                correction(i, &long, Some(ActivityCategory::Creating), Some(FocusLevel::Deep))
            })
            .collect();
        let block = build_corrections_block(&corrections);
        assert!(block.contains("USER CORRECTIONS"));
        assert!(
            block.chars().count() < CORRECTION_FEEDBACK_CHAR_CAP + 800,
            "block stays bounded by the char cap (+header/one-line slack)"
        );
    }

    #[test]
    fn truncates_long_text_on_char_boundary() {
        let text = "a".repeat(ITEM_TEXT_CHAR_CAP + 50);
        let truncated = truncate_chars(&text, ITEM_TEXT_CHAR_CAP);
        // cap chars + the ellipsis.
        assert_eq!(truncated.chars().count(), ITEM_TEXT_CHAR_CAP + 1);
    }

    #[test]
    fn supersedes_id_defaults_to_none_when_absent() {
        // ADR 0046: a draft with no supersedes_id (the common case) parses to None,
        // exactly like reinforces_id — the `#[serde(default)]` must hold.
        let json = r#"{"subject":"Rust","statement":"Rust is compiled","support_refs":[1,2]}"#;
        let parsed: DistilledConclusion = serde_json::from_str(json).expect("parse");
        assert_eq!(parsed.supersedes_id, None);
        assert_eq!(parsed.reinforces_id, None);
        // And it round-trips when present.
        let with = r#"{"subject":"Rust","statement":"x","support_refs":[1],"supersedes_id":7}"#;
        let parsed: DistilledConclusion = serde_json::from_str(with).expect("parse");
        assert_eq!(parsed.supersedes_id, Some(7));
    }

    #[test]
    fn dismissed_block_marks_supersede_rows_as_already_replaced() {
        // ADR 0046 Slice 3H: user Dismiss rows read as "rejected", machine
        // supersede rows as "already replaced"; both stay in the block.
        let mut user = dismissal("Rust", "Rust is interpreted", "1,2", 2);
        user.source = "user".to_string();
        let mut sup = dismissal("Go", "Go has classes", "3,4", 2);
        sup.source = "supersede".to_string();
        let block = build_dismissed_conclusions_block(&[user, sup]);
        assert!(block.contains("(rejected) subject=Rust"));
        assert!(block.contains("(already replaced) subject=Go"));
    }

    #[test]
    fn filter_known_refs_drops_unknown_and_dedups() {
        let valid: std::collections::HashSet<i64> = [1, 2, 3].into_iter().collect();
        // 9 is not in the set; the second `2` is a duplicate; order preserved.
        let filtered = filter_known_refs(&[2, 9, 1, 2, 3], &valid);
        assert_eq!(filtered, vec![2, 1, 3]);
        // No valid refs at all => empty (the caller skips ungrounded conclusions).
        assert!(filter_known_refs(&[9, 10], &valid).is_empty());
    }

    fn dismissal(subject: &str, statement: &str, fingerprint: &str, count: i64) -> DismissalState {
        DismissalState {
            subject: subject.to_string(),
            statement: statement.to_string(),
            evidence_fingerprint: fingerprint.to_string(),
            evidence_activity_count: count,
            dismissed_at_ms: 1_000,
            source: "user".to_string(),
        }
    }

    #[test]
    fn matching_dismissal_is_case_insensitive_on_subject_and_statement() {
        let dismissals = vec![dismissal("Apple", "Interested in Apple", "1,2", 2)];
        // Exact match on both, case-insensitively.
        assert!(matching_dismissal(&dismissals, "apple", "INTERESTED IN APPLE").is_some());
        // A different statement (even same subject) does not match.
        assert!(matching_dismissal(&dismissals, "Apple", "Loves Apple").is_none());
        // A different subject does not match.
        assert!(matching_dismissal(&dismissals, "Rust", "Interested in Apple").is_none());
    }

    fn authored(id: i64, text: &str, topic: Option<&str>) -> AuthoredContext {
        AuthoredContext {
            id,
            text: text.to_string(),
            topic: topic.map(str::to_string),
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        }
    }

    #[test]
    fn authored_context_block_is_empty_without_statements() {
        assert!(build_authored_context_block(&[]).is_empty());
        // All-blank statements also yield no block.
        assert!(build_authored_context_block(&[authored(1, "   ", None)]).is_empty());
    }

    #[test]
    fn authored_context_block_labels_statements_and_topics() {
        let items = vec![
            authored(2, "I care about typography", None),
            authored(1, "I'm a designer", Some("role")),
        ];
        let block = build_authored_context_block(&items);
        assert!(block.contains("USER-AUTHORED CONTEXT"));
        assert!(block.contains("ASSERTED by the user"));
        assert!(block.contains("- I care about typography"));
        assert!(block.contains("- (topic: role) I'm a designer"));
        // Never cited as Activity evidence.
        assert!(block.contains("do not cite them as support_refs"));
    }

    #[test]
    fn authored_context_block_respects_char_cap() {
        // Many long statements; the block must stay bounded by the cap (plus the
        // single header + one over-budget line that the !lines.is_empty() guard
        // allows once).
        let long = "x".repeat(300);
        let items: Vec<AuthoredContext> = (0..50)
            .map(|i| authored(i, &long, None))
            .collect();
        let block = build_authored_context_block(&items);
        // The header is always present; the statement body stays near the cap and
        // does not include all 50 * 300 chars.
        assert!(block.contains("USER-AUTHORED CONTEXT"));
        assert!(
            block.chars().count() < AUTHORED_CONTEXT_CHAR_CAP + 600,
            "block stays bounded by the char cap (+header/one-line slack)"
        );
    }

    #[test]
    fn dismissed_conclusions_block_is_empty_without_dismissals() {
        assert!(build_dismissed_conclusions_block(&[]).is_empty());
    }

    #[test]
    fn dismissed_conclusions_block_lists_each_rejection() {
        let dismissals = vec![
            dismissal("Apple", "Interested in Apple", "1,2", 2),
            dismissal("Vim", "Prefers Vim", "3,4", 2),
        ];
        let block = build_dismissed_conclusions_block(&dismissals);
        assert!(block.contains("DISMISSED CONCLUSIONS"));
        assert!(block.contains("subject=Apple statement=Interested in Apple"));
        assert!(block.contains("subject=Vim statement=Prefers Vim"));
        // The instruction enforces the high-bar / never-same-evidence rule.
        assert!(block.contains("substantially MORE and NEWER"));
    }

    #[test]
    fn known_subjects_block_is_empty_without_handles() {
        let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
        assert!(build_known_subjects_block(&[], &empty).is_empty());
        // All-blank handles also yield no block (prompt unchanged).
        assert!(build_known_subjects_block(&["   ".to_string()], &empty).is_empty());
    }

    #[test]
    fn known_subjects_block_renders_handles_and_header() {
        let handles = vec!["Apple".to_string(), "async communication".to_string()];
        let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
        let block = build_known_subjects_block(&handles, &empty);
        assert!(block.contains("KNOWN SUBJECTS"));
        // The reuse-verbatim instruction frames the block.
        assert!(block.contains("VERBATIM"));
        assert!(block.contains("reinforces the existing subject"));
        // One `- {handle}` line per handle, in order.
        assert!(block.contains("- Apple"));
        assert!(block.contains("- async communication"));
    }

    #[test]
    fn known_subjects_block_nests_conclusions_in_order() {
        let handles = vec!["Apple".to_string()];
        let mut map: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
        // Confidence-desc order is the caller's job; the block preserves push order.
        map.insert(
            "Apple".to_string(),
            vec![
                (7, "Interested in Apple silicon".to_string()),
                (9, "Follows Apple keynotes".to_string()),
            ],
        );
        let block = build_known_subjects_block(&handles, &map);
        assert!(block.contains("- Apple\n"));
        // Indented `id: statement` lines nested under the handle, in order.
        let first = block.find("    7: Interested in Apple silicon").unwrap();
        let second = block.find("    9: Follows Apple keynotes").unwrap();
        assert!(first < second, "conclusion lines keep their given order");
        // The instruction explains the reinforces-by-id semantics.
        assert!(block.contains("reinforces_id"));
    }

    #[test]
    fn known_subjects_block_degrades_overflowing_subject_to_handle_only() {
        // Two subjects, each with one very long conclusion line. Sized so the first
        // subject's full block fits but the second's conclusion line overflows the cap,
        // forcing the second down to handle-only.
        let long = "x".repeat(KNOWN_SUBJECTS_CHAR_CAP * 3 / 4);
        let handles = vec!["First".to_string(), "Second".to_string()];
        let mut map: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
        map.insert("First".to_string(), vec![(1, long.clone())]);
        map.insert("Second".to_string(), vec![(2, long)]);
        let block = build_known_subjects_block(&handles, &map);
        // First subject keeps its conclusion line...
        assert!(block.contains("    1: xxx"), "first subject's conclusion shown");
        // ...but the second degrades to handle-only (its `2:` line dropped).
        assert!(block.contains("- Second\n"), "second handle still present");
        assert!(
            !block.contains("    2: "),
            "overflowing subject degrades to handle-only"
        );
    }

    #[test]
    fn known_subjects_block_respects_char_cap() {
        // Many handles; the block stays bounded by the cap (plus the header + the
        // single over-budget line the lines.is_empty() guard allows once).
        let handles: Vec<String> = (0..2_000).map(|i| format!("subject-{i:05}")).collect();
        let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
        let block = build_known_subjects_block(&handles, &empty);
        assert!(block.contains("KNOWN SUBJECTS"));
        assert!(
            block.chars().count() < KNOWN_SUBJECTS_CHAR_CAP + 700,
            "block stays bounded by the char cap (+header/one-line slack)"
        );
    }

    #[test]
    fn conclusion_preamble_instructs_reuse_of_known_subject_handles() {
        let preamble = conclusion_preamble();
        assert!(preamble.contains("KNOWN SUBJECTS"));
        assert!(preamble.contains("reuse that handle VERBATIM"));
        assert!(preamble.contains("genuinely new subject"));
    }

    #[test]
    fn resurface_gate_drops_same_evidence_and_below_bar() {
        // The fresh fingerprint for the same evidence set matches the dismissal's,
        // so the same-evidence drop fires regardless of count.
        let d = dismissal("Apple", "Interested in Apple", &evidence_fingerprint(&[1, 2]), 2);
        let dismissals = vec![d];

        // Same evidence as rejected → must be recognized as a same-evidence match.
        let same = evidence_fingerprint(&[2, 1]);
        let matched = matching_dismissal(&dismissals, "apple", "interested in apple")
            .expect("matches");
        assert_eq!(same, matched.evidence_fingerprint, "same evidence => drop");

        // Different (more) evidence: fingerprint differs, and the resurface bar
        // gates on the support count vs the prior 2 (needs ≥ 4 at 2.0×).
        assert_ne!(evidence_fingerprint(&[1, 2, 3, 4]), matched.evidence_fingerprint);
        assert!(!confidence::meets_resurface_bar(3, matched.evidence_activity_count));
        assert!(confidence::meets_resurface_bar(4, matched.evidence_activity_count));
    }

    #[test]
    fn distillation_uses_the_confidence_policy_formation_bar() {
        // The formation bar (#95) gates distillation: a single supporting
        // Activity is below the bar and must not form a Conclusion.
        assert!(!confidence::meets_formation_bar(1));
        assert!(confidence::meets_formation_bar(2));
        // And the wired initial_confidence rises with support, drops on contradiction.
        assert!(
            confidence::initial_confidence(3, 0) > confidence::initial_confidence(2, 0)
        );
        assert!(
            confidence::initial_confidence(3, 1) < confidence::initial_confidence(3, 0)
        );
    }
}
