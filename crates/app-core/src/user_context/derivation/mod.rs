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

use app_infra::user_context::{confidence, guardrail};
use app_infra::{
    evidence_fingerprint, ActivityCorrection, CaptureWindow, NewActivity, NewActivityEvidence,
    NewConclusion, NewConclusionEvidence, NewDerivationRun, SubjectVectorStore, SupersedeOutcome,
    UserContextStore,
};

use super::activity_spans::{place, too_thin_for_summary, Placement, Span};
use super::continuation::CONTINUATION_GAP_MS;
use super::evidence_tags::strip_evidence_tags;

mod distillation;
mod prompt_blocks;
pub use distillation::*;
pub(crate) use prompt_blocks::*;

/// System instruction for the Activity-segmentation pass. Kept terse: the
/// detailed item formatting + the return shape live in the per-call prompt and
/// the `DerivedActivityBatch` JSON schema.
const ACTIVITY_PREAMBLE: &str = "You analyze a chronological stretch of a single user's captured \
on-screen text and spoken transcripts and segment it into semantic Activity episodes. An \
Activity is a coherent unit of work or intent — its boundaries are INTENT SHIFTS (for example \
\"stopped wrestling the deploy, started writing the design doc\"), NOT app switches or fixed time \
windows. A single Activity may span multiple apps, and a single app may host several Activities. \
Do not emit one Activity per app or per time slice. Items about the same video, page or topic \
belong to ONE Activity even when the user alternates between watching it and reading about it; \
never return two Activities about the same subject that overlap in time. For each Activity give a short title, a one \
or two sentence summary of what the user was doing and how, and an optional category from this \
fixed taxonomy or omit it when unsure: creating (producing anything — code, documents, designs, \
slides, music), communication (asynchronous text — email, chat, messages), meetings (real-time \
conversation the user takes part in — calls, video meetings; never a video, podcast or stream they \
only watch), research (reading or searching in service of the current task), learning (deliberate skill-building for its own sake — courses, tutorials), organizing \
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
summaries to the work/task itself. Write titles and summaries without a subject, starting with a \
verb; never write 'the user', 'they' or 'their'; name the project or site. The f<id>/a<id> tags go \
only in evidence_refs and headline_ref, never in a title or summary. Return the structured result.";

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
    /// The id of an ONGOING ACTIVITY (a neighbouring window's Activity shown in
    /// the prompt) this one carries on. Set → that row is extended instead of a
    /// new one inserted; an id that was not shown is ignored (insert).
    #[serde(default)]
    pub continues_id: Option<i64>,
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

/// The outcome of one [`derive_activities`] call, whose `completed` run row
/// (count + estimated tokens) is already written with the Activities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityDerivationOutcome {
    /// Number of Activities inserted into the store.
    pub inserted: usize,
    /// Number of neighbouring Activities extended instead (continuation).
    pub extended: usize,
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

/// Map an audio item's `source_kind` to the prompt tag telling the model WHO
/// spoke: `you` = the user's own voice (microphone), `computer` = sound the Mac
/// played (system audio: a call's other side OR a video/podcast). `None` for
/// frames or an unknown/absent source. The ADR 0050 attribution signal; not
/// `other-side`, which made the model read every video as a conversation.
fn source_tag(subject_type: &str, source_kind: Option<&str>) -> Option<&'static str> {
    if subject_type != "audio_segment" {
        return None;
    }
    match source_kind {
        Some("microphone") => Some("you"),
        Some("system_audio") => Some("computer"),
        _ => None,
    }
}

/// The Nth ephemeral speaker-label suffix: 0→`A`, 1→`B`, … 25→`Z`, then a numeric
/// fallback for the (never-in-practice) 27th+ distinct voice in one segment.
fn speaker_label(index: usize) -> String {
    if index < 26 {
        ((b'A' + index as u8) as char).to_string()
    } else {
        // ponytail: >26 distinct speakers in one segment does not happen; number it.
        format!("#{}", index + 1)
    }
}

/// Render diarized speaker turns as anonymous per-item "Speaker A/B/…" lines,
/// mapping each distinct `cluster_id` to an ephemeral label in first-appearance
/// order (labels are transient, per-item, never stored — like the cited-id tags).
///
/// PRIVACY BY CONSTRUCTION (ADR 0050): the input is `(cluster_id, text)` ONLY —
/// there is NO name field — so a recognized person's display name is
/// unrepresentable here and cannot leak into the prompt handed to a (possibly
/// cloud) engine. Names resolve on-device only. Empty string when there are no
/// turns (the flat transcript text stays the only audio content in that case).
fn render_speaker_turns(turns: &[(i64, String)]) -> String {
    let mut labels: HashMap<i64, String> = HashMap::new();
    let mut out = String::new();
    for (cluster_id, text) in turns {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let next = labels.len();
        let label = labels
            .entry(*cluster_id)
            .or_insert_with(|| format!("Speaker {}", speaker_label(next)))
            .clone();
        out.push_str(&format!("{label}: {text}\n"));
    }
    out
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
pub(crate) fn format_utc_ms(ms: i64) -> String {
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

/// Format signed timezone offset MINUTES as `UTC+05:30` / `UTC-08:00`.
fn offset_label(offset_min: i64) -> String {
    let sign = if offset_min < 0 { '-' } else { '+' };
    let abs = offset_min.abs();
    format!("UTC{}{:02}:{:02}", sign, abs / 60, abs % 60)
}

/// Format a unix-millis instant as a local wall-clock string with an explicit
/// offset suffix (`2026-06-11 22:12 UTC+05:30`). Mirrors [`format_utc_ms`] but
/// shifts the instant by `offset_ms` (offset in MILLIseconds) before rendering.
fn format_local_ms(ms: i64, offset_ms: i64) -> String {
    let dt =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms + offset_ms) * 1_000_000)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let clock = dt.time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} {}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        clock.hour(),
        clock.minute(),
        offset_label(offset_ms / 60_000),
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
its capture time, and (when known) the app/URL it came from. All times are UTC. Audio (a<id>) \
items also carry a speaker source tag: source=you means the user's OWN voice (microphone), \
source=computer means sound the computer played (system audio) — the other side of a call, OR a \
video, podcast, stream or music. Decide which from the frames around it: it is a conversation only \
when a call or meeting app is on screen or the user's own voice answers it; otherwise it is media \
the user is watching or listening to, and it belongs in the same Activity as the frames showing \
that media. Attribute speech by this tag — never render computer audio as words the user said, \
and never render the user's words as someone else's. The tags are for you: write \"watched a video \
about X\", never \"computer audio\" or \"computer-played\", in a title or summary. An audio item may list \"Speaker A\"/\"Speaker B\"/… lines beneath its text: those are the \
distinct anonymized voices within that one segment (the labels are local to that item only, not \
identities). Segment these items into Activity episodes by intent shift and return \
DerivedActivityBatch.\n\n",
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
        if let Some(source) = source_tag(&item.subject_type, item.source_kind.as_deref()) {
            prompt.push_str(&format!(" source={source}"));
        }
        if let Some(app) = item.app_label.as_deref().filter(|s| !s.trim().is_empty()) {
            prompt.push_str(&format!(" app={app}"));
        }
        if let Some(url) = item.url.as_deref().filter(|s| !s.trim().is_empty()) {
            prompt.push_str(&format!(" url={url}"));
        }
        prompt.push('\n');
        let text = truncate_chars(item.text.trim(), ITEM_TEXT_CHAR_CAP);
        prompt.push_str(&text);
        prompt.push('\n');
        // Anonymous per-speaker turn structure beneath the flat transcript (ADR
        // 0050). Empty for frames and for audio with no diarized turns. Bounded by
        // the same ITEM_TEXT_CHAR_CAP as the flat text: the turns re-render the
        // same transcript, so without this cap one long segment's turns would
        // defeat the per-item prompt budget the flat-text cap exists to enforce.
        prompt.push_str(&truncate_chars(
            &render_speaker_turns(&item.speaker_turns),
            ITEM_TEXT_CHAR_CAP,
        ));
        prompt.push('\n');
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
/// `completed_run` is the caller's `completed` run row (kind, window,
/// provider/model labels); this fn fills in the count and token estimates and
/// writes it in the SAME transaction as the Activities, so a window is never
/// half-saved (Activities without the run that marks it done). On `Err`
/// nothing was written and the caller records the `failed` run.
pub async fn derive_activities(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    window: CaptureWindow,
    mut completed_run: NewDerivationRun,
) -> Result<ActivityDerivationOutcome, String> {
    if window.items.is_empty() {
        store
            .insert_derivation_run(completed_run)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(ActivityDerivationOutcome {
            inserted: 0,
            extended: 0,
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

    // Activities neighbouring windows already wrote, offered for continuation.
    let neighbours =
        super::continuation::neighbours_for_window(store, window.start_ms, window.end_ms).await?;
    let neighbour_ids: std::collections::HashSet<i64> = neighbours.iter().map(|a| a.id).collect();

    let mut prompt = build_prompt(&window);
    prompt.push_str(&super::continuation::neighbour_block(&neighbours));
    prompt.push_str(&build_corrections_block(&corrections));
    let input_tokens = estimate_tokens(ACTIVITY_PREAMBLE) + estimate_tokens(&prompt);

    let batch: DerivedActivityBatch = ai_engine::extract_with_preamble::<DerivedActivityBatch>(
        engine,
        ACTIVITY_PREAMBLE,
        &prompt,
    )
    .await
    .map_err(|error| error.to_string())?;

    let output_tokens = serde_json::to_string(&batch)
        .map(|json| estimate_tokens(&json))
        .unwrap_or(0);

    completed_run.input_tokens = input_tokens;
    completed_run.output_tokens = output_tokens;
    let (inserted, extended) = persist_batch(
        store,
        &window,
        &tag_to_captured_at,
        &neighbour_ids,
        &batch,
        completed_run,
    )
    .await?;
    Ok(ActivityDerivationOutcome {
        inserted,
        extended,
        input_tokens,
        output_tokens,
    })
}

/// Persist one engine batch for a NON-EMPTY window plus its `completed_run`
/// row in one transaction, returning `(inserted, extended)`. A batch that
/// grounds nothing (no activities, or all ungrounded) is an `Err` with nothing
/// written, so the worker records the window `failed` and the Retry pass
/// re-derives it instead of a `completed` run hiding the hole.
async fn persist_batch(
    store: &UserContextStore,
    window: &CaptureWindow,
    tag_to_captured_at: &HashMap<String, i64>,
    neighbour_ids: &std::collections::HashSet<i64>,
    batch: &DerivedActivityBatch,
    mut completed_run: NewDerivationRun,
) -> Result<(usize, usize), String> {
    let mut drafts: Vec<(Option<i64>, NewActivity)> = Vec::new();
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

        let draft = NewActivity {
            title: strip_evidence_tags(activity.title.trim(), tag_to_captured_at),
            summary: strip_evidence_tags(activity.summary.trim(), tag_to_captured_at),
            category: parse_category(&activity.category),
            focus: parse_focus(&activity.focus),
            started_at_ms,
            ended_at_ms,
            derivation_run_id: None,
            evidence,
        };

        let continues = activity
            .continues_id
            .filter(|id| neighbour_ids.contains(id));
        drafts.push((continues, draft));
    }

    // Place oldest first (longer first on a tie) against the time stored
    // Activities and this batch's earlier writes already hold, so no two
    // Activities overlap (see `activity_spans`).
    drafts.sort_by_key(|(_, d)| (d.started_at_ms, d.started_at_ms - d.ended_at_ms));
    let mut claimed: Vec<(i64, Span)> = store
        .list_activities_in_range(
            window.start_ms.saturating_sub(CONTINUATION_GAP_MS),
            window.end_ms.saturating_add(CONTINUATION_GAP_MS),
        )
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|a| (a.id, (a.started_at_ms, a.ended_at_ms)))
        .collect();
    // Every write below plus the run row commit together (dropped on `Err` or
    // a quit mid-loop, it rolls back).
    let mut write = store
        .begin_activity_window_write()
        .await
        .map_err(|error| error.to_string())?;
    let mut inserted = 0usize;
    let mut extended = 0usize;
    for (continues, mut draft) in drafts {
        // Capture deleted during the AI call grounds nothing (ADR 0029).
        write
            .retain_live_evidence(&mut draft.evidence)
            .await
            .map_err(|error| error.to_string())?;
        let times = draft.evidence.iter().filter_map(|e| e.captured_at_ms);
        let (Some(start), Some(end)) = (times.clone().min(), times.max()) else {
            continue;
        };
        (draft.started_at_ms, draft.ended_at_ms) = (start, end);
        let span = (draft.started_at_ms, draft.ended_at_ms);
        // Continuation: extend the neighbour (blank title or summary keep the
        // stored ones). Anything else is a new row over the time `place` gave it.
        match place(span, continues, &claimed) {
            Placement::Insert(piece) => (draft.started_at_ms, draft.ended_at_ms) = piece,
            Placement::Extend(id) => {
                if write
                    .extend_activity(id, &draft)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    if let Some(held) = claimed.iter_mut().find(|c| c.0 == id) {
                        held.1 = (held.1 .0.min(span.0), held.1 .1.max(span.1));
                    }
                    extended += 1;
                    continue;
                }
                // A vanished row falls through to insert.
            }
        }
        if draft.title.is_empty() {
            draft.title = "Activity".to_string();
        }
        if too_thin_for_summary(&draft.evidence) {
            draft.summary.clear();
        }
        let stored = (draft.started_at_ms, draft.ended_at_ms);
        let id = write
            .insert_activity(&draft)
            .await
            .map_err(|error| error.to_string())?;
        claimed.push((id, stored));
        inserted += 1;
    }

    // ponytail: no minimum-item guard; add `window.items.len() >= 3` if
    // near-empty windows start reading "failed". (Lock-screen-only windows
    // can't: `read_capture_window` drops lock-screen frames, so they read as
    // empty and are skipped before reaching here.)
    if inserted + extended == 0 {
        return Err("The model returned no Activities grounded in this window.".to_string());
    }
    completed_run.activities_derived = inserted as i64;
    write
        .commit_with_run(completed_run)
        .await
        .map_err(|error| error.to_string())?;
    Ok((inserted, extended))
}

// === #94: Conclusion distillation =========================================

#[cfg(test)]
mod tests;
