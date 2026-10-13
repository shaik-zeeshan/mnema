//! **User Context Digest** generation (issue #89): the engine-written narrative
//! lede the Insights Overview shows above the story feed for one local-calendar
//! range (day / week / month), plus its short generated headline ("A deep week
//! in the editor") rendered in large type above the prose.
//!
//! Lazy + cached: the `get_user_context_digest` command (in `commands.rs`) calls
//! [`get_or_generate_digest`] when the user views a range. A stored digest whose
//! [`app_infra::digest_input_fingerprint`] still matches the range's current
//! Activity set is returned without any engine call — the common path. A stale
//! fingerprint regenerates only past a per-range **freshness floor**
//! ([`freshness_floor_ms`]): ranges containing "now" churn on every worker beat
//! (each new Activity flips the day, week, AND month fingerprints), so a recent
//! digest is served slightly stale rather than re-billing the engine per visit.
//! Past ranges are unaffected — their fingerprints simply keep matching. Only a
//! changed-and-old (or missing) input set pays for one structured-extraction
//! round trip, mirroring `derivation.rs`: same engine resolution, same guardrail
//! soft preamble, same estimated-token `derivation_run` bookkeeping (kind
//! `'digest'`). Delete Recent Capture stays immediate: it purges overlapping
//! digest rows outright, so there is no stale row for the floor to serve.
//!
//! Concurrency: a double-click / range flicker may race two generations for the
//! same range. That is acceptable — `upsert_digest` is idempotent per
//! `(range_kind, range_start_ms)` (last writer wins, both writers derived from
//! the same input set) — so there is deliberately no in-flight lock here; the
//! module has no established guard pattern to reuse either.

use capture_types::{
    Activity, ActivityCategory, AiRuntimeSettings, DigestAbsentReason, FocusLevel,
    UserContextDigest, UserContextDigestResult,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use std::collections::HashMap;

use app_infra::user_context::guardrail;
use app_infra::{digest_input_fingerprint, NewDerivationRun, StoredDigest, UserContextStore};

use super::derivation::{category_label, estimate_tokens, truncate_chars};
use super::worker::{model_label_for, now_ms, provider_label_for};

/// Base system instruction for the Digest pass. Like the Conclusion preamble,
/// the engine never sees this bare: [`digest_preamble`] prepends the soft
/// Sensitive Category Guardrail instruction first.
const DIGEST_PREAMBLE_BASE: &str = "You write a short narrative digest of a single user's derived \
Activity episodes over one calendar range (a day, a week, or a month). Write 2 to 4 sentences of \
plain prose addressed to the user in the second person — for example \"Most of this week went into \
the billing migration…\". Mention the dominant work, the shape it took across the days, and any \
notable shift in focus or topic. Do NOT use markdown, lists, bullet points, headers, or quotes — \
only sentences. Ground every claim in the listed Activities; do not invent work that is not shown. \
Each Activity line carries its local clock times; cite them when they help (\"before the late-morning \
call\"). Weigh the range by time: lead with what took the most of it (the time-by-category line \
shows the real shape), and never call an episode of a few seconds focus, work or a session. Keep \
watching or listening (videos, podcasts, streams) apart from doing (building, writing, calls): \
say \"you watched a video about X\", never \"you built X\" or \"a conversation about X\", unless \
an Activity shows the user doing it. Be specific — name the project, the video's subject, the \
site or app — rather than a generic topic. Also write a headline for the range: a full sentence (or two short ones) in sentence \
case, concrete and grounded in the dominant work — for example \"A build week. The capture \
pipeline ate your mornings\". Never put a dash in the headline. Optionally add a turn: a short \
clause naming what changed the shape of the range, read as a continuation of the headline — for \
example \"until Thursday's calls took the whole day back\". Leave the turn empty when nothing \
turned. Keep the headline under 70 characters and the headline and turn together under 120. No clickbait, no terminal punctuation on the headline, no surrounding quotes. \
Return the structured result.";

/// The full Digest preamble the engine sees: the **soft** Sensitive Category
/// Guardrail instruction prepended to [`DIGEST_PREAMBLE_BASE`], exactly as
/// `derivation.rs`'s `conclusion_preamble` does — the off-limits-category rule
/// frames the task before the engine reads what to produce.
fn digest_preamble() -> String {
    format!(
        "{}\n\n{}",
        guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION,
        DIGEST_PREAMBLE_BASE
    )
}

/// A narrative over fewer than this many Activities is silly — return no digest.
const MIN_DIGEST_ACTIVITIES: usize = 2;

/// Freshness floor per range kind: a cached digest younger than this is served
/// even when the range's Activity fingerprint changed, rate-limiting how often
/// a churning current-day/week/month range re-bills the engine. Wider ranges
/// get wider floors because their prompts are the largest and a few hours of
/// missing tail matters proportionally less to the narrative.
fn freshness_floor_ms(range_kind: &str) -> i64 {
    const HOUR_MS: i64 = 60 * 60 * 1000;
    match range_kind {
        "day" => HOUR_MS,
        "week" => 6 * HOUR_MS,
        // "month" — the only other kind past the entry validation.
        _ => 24 * HOUR_MS,
    }
}

/// Whether a stored digest generated at `generated_at_ms` is still inside its
/// range kind's freshness floor at `at_ms`. A clock that moved backwards
/// (negative age) counts as fresh rather than forcing a regeneration.
fn within_freshness_floor(range_kind: &str, generated_at_ms: i64, at_ms: i64) -> bool {
    at_ms.saturating_sub(generated_at_ms) < freshness_floor_ms(range_kind)
}

/// How many recent corrections to scan when deciding whether the freshness floor
/// should be bypassed. A correction older than the most recent 30 cannot be
/// newer than a just-generated digest in any realistic scenario, so this bound
/// (mirroring `derivation.rs`'s `CORRECTION_FEEDBACK_LIMIT`) is ample.
const CORRECTION_SCAN_LIMIT: i64 = 30;

/// Whether any Activity in `activities` was corrected (#108) at a time strictly
/// after `generated_at_ms` — i.e. the user relabeled an in-range Activity since
/// the stored digest was generated. Used to let a user correction bypass the
/// freshness floor (PR #112 digest edge): otherwise a corrected-away label can
/// stay visible in a stale narrative for up to the floor window.
///
/// Best-effort: a store read failure returns `false` (do not force regeneration
/// on a transient error — the floor still rate-limits, and the next tick retries).
async fn range_has_correction_after(
    store: &UserContextStore,
    activities: &[Activity],
    generated_at_ms: i64,
) -> bool {
    let Ok(corrections) = store.list_activity_corrections(CORRECTION_SCAN_LIMIT).await else {
        return false;
    };
    let in_range: std::collections::HashSet<i64> = activities.iter().map(|a| a.id).collect();
    corrections
        .iter()
        .any(|c| c.corrected_at_ms > generated_at_ms && in_range.contains(&c.activity_id))
}

/// The `Err` a digest held back by the hard sensitive post-filter returns: a
/// stable code the frontend recognises ("held back by the sensitive filter"),
/// distinct from a `too_few` absence and from a real failure.
pub const DIGEST_SENSITIVE_HOLD: &str = "digest_sensitive_hold";

/// How long a failed (or sensitive-held) generation is replayed to the lazy
/// path instead of re-billing the engine on every Overview visit.
const FAILURE_COOLDOWN_MS: i64 = 15 * 60 * 1000;

/// `(range_kind, range_start_ms)` → `(input fingerprint, failed_at_ms, message)`.
// ponytail: in-memory only — a restart forgets cooldowns, which just costs one
// extra engine call. Nothing persisted, no migration.
type CooldownMap = HashMap<(String, i64), (String, i64, String)>;
static FAILURE_COOLDOWN: std::sync::LazyLock<std::sync::Mutex<CooldownMap>> =
    std::sync::LazyLock::new(Default::default);

/// The stored failure to replay for this range, if the lazy path should skip
/// the engine. A re-read (`force`) bypasses it; a changed fingerprint or an
/// expired window clears it.
fn cooldown_replay(
    range_kind: &str,
    range_start_ms: i64,
    fingerprint: &str,
    force: bool,
    at_ms: i64,
) -> Option<String> {
    if force {
        return None;
    }
    let mut map = FAILURE_COOLDOWN.lock().unwrap_or_else(|e| e.into_inner());
    let key = (range_kind.to_string(), range_start_ms);
    match map.get(&key) {
        Some((fp, failed_at, message))
            if fp == fingerprint && at_ms.saturating_sub(*failed_at) < FAILURE_COOLDOWN_MS =>
        {
            Some(message.clone())
        }
        Some(_) => {
            map.remove(&key);
            None
        }
        None => None,
    }
}

/// Record a failed outcome for the cooldown and hand the message back as `Err`.
fn cooldown_fail<T>(
    range_kind: &str,
    range_start_ms: i64,
    fingerprint: &str,
    at_ms: i64,
    message: String,
) -> Result<T, String> {
    FAILURE_COOLDOWN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            (range_kind.to_string(), range_start_ms),
            (fingerprint.to_string(), at_ms, message.clone()),
        );
    Err(message)
}

fn cooldown_clear(range_kind: &str, range_start_ms: i64) {
    FAILURE_COOLDOWN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(range_kind.to_string(), range_start_ms));
}

/// Per-activity summary cap inside the Digest prompt, so one verbose Activity
/// cannot dominate the budget. Mirrors `derivation.rs`'s per-item-cap approach
/// (`ACTIVITY_SUMMARY_CHAR_CAP` there is module-private, so the value is
/// restated here); kept smaller because a month range can hold many Activities.
const DIGEST_SUMMARY_CHAR_CAP: usize = 240;

/// Total char budget for the rendered Activity lines. Mirrors how
/// `derivation.rs` bounds its prompt inputs (per-item caps + a bounded item
/// count via `MAX_ITEMS`); expressed here as a char budget because the input is
/// a whole calendar range rather than a capture window. ~6k estimated tokens.
///
/// We do NOT drop the oldest Activities when over budget. Instead the prompt
/// **degrades by recency** ([`build_digest_prompt`]): the newest episodes stay
/// at full detail, earlier ones collapse to a one-line compact form, and the
/// oldest are summarized one line per local day (a cached day-digest narrative
/// when one exists, else computed stats). The whole range is always
/// represented, just at decreasing detail going back in time.
const DIGEST_PROMPT_CHAR_CAP: usize = 24_000;

/// Sample-title char cap inside a computed rollup line: long titles are
/// shortened so two of them fit on one day line.
const ROLLUP_TITLE_CHAR_CAP: usize = 40;

/// How many top categories a computed rollup line names before stopping.
const ROLLUP_TOP_CATEGORIES: usize = 3;

/// How many sample titles a computed rollup line folds in.
const ROLLUP_SAMPLE_TITLES: usize = 2;

/// The structured result the engine returns for one Digest pass. Flat (two
/// string fields), so the schema is `$ref`/`$defs`-free without `#[schemars(inline)]`;
/// the reference-free guard test below keeps it that way.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DigestNarrative {
    /// 2–4 sentences of second-person plain prose.
    pub narrative: String,
    /// Full-sentence headline above the prose, with no dash in it. Defaulted
    /// (same stance as `derivation.rs`'s optional fields) so an engine that
    /// omits it still yields a narrative-only digest instead of an extraction
    /// failure; [`joined_headline`] turns empty/whitespace into `None`.
    #[serde(default)]
    pub headline: String,
    /// Optional clause continuing the headline ("until Thursday's calls took
    /// the whole day back"); empty when nothing turned. Stored joined onto the
    /// headline as `headline — turn` (no column of its own).
    #[serde(default)]
    pub turn: String,
}

/// snake_case label for a [`FocusLevel`] in the Digest prompt.
fn focus_label(focus: FocusLevel) -> &'static str {
    match focus {
        FocusLevel::Deep => "deep",
        FocusLevel::Mixed => "mixed",
        FocusLevel::Distracted => "distracted",
    }
}

const DAY_MS: i64 = 86_400_000;

/// Recover the local UTC-offset (in ms) from the frontend-computed range start.
///
/// The frontend computes the half-open digest window on the LOCAL calendar, so
/// `range_start_ms` is a local midnight: `(range_start_ms + offset) % day == 0`.
/// That pins the offset modulo 24h; the representative is chosen in
/// `(-10h, +14h]`, which covers every real-world offset except UTC−11/−12
/// (those label days one weekday late — a cosmetic prompt-label skew only,
/// never a correctness issue, since the label feeds nothing but the narrative).
fn recover_local_offset_ms(range_start_ms: i64) -> i64 {
    let offset = (-range_start_ms).rem_euclid(DAY_MS); // in [0, 24h)
    if offset > 14 * 3_600_000 {
        offset - DAY_MS
    } else {
        offset
    }
}

/// Local-midnight (UTC ms) of the local day containing `at_ms`, using the
/// offset recovered from the range start. The result matches the
/// `range_start_ms` a DAY-kind [`StoredDigest`] is keyed on (the frontend
/// computes those starts the same way), so it is the join key for reusing a
/// cached day-digest narrative in a rollup line.
///
/// Same `div_euclid`/`rem_euclid`-flavoured arithmetic the [`day_label`] /
/// [`recover_local_offset_ms`] pair uses: shift into local time, floor to the
/// day grid, shift back. Saturating adds keep hostile timestamps from panicking.
fn local_day_start_ms(at_ms: i64, local_offset_ms: i64) -> i64 {
    let shifted = at_ms.saturating_add(local_offset_ms);
    let local_midnight = shifted.div_euclid(DAY_MS).saturating_mul(DAY_MS);
    local_midnight.saturating_sub(local_offset_ms)
}

/// Compact local day label ("Tue Jun 9") for one Activity start time, using the
/// offset recovered from the range start. Falls back to the epoch date on an
/// out-of-range timestamp (never panics on hostile data).
fn day_label(at_ms: i64, local_offset_ms: i64) -> String {
    let shifted_seconds = (at_ms.saturating_add(local_offset_ms)).div_euclid(1_000);
    let date = OffsetDateTime::from_unix_timestamp(shifted_seconds)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .date();
    let weekday = match date.weekday() {
        time::Weekday::Monday => "Mon",
        time::Weekday::Tuesday => "Tue",
        time::Weekday::Wednesday => "Wed",
        time::Weekday::Thursday => "Thu",
        time::Weekday::Friday => "Fri",
        time::Weekday::Saturday => "Sat",
        time::Weekday::Sunday => "Sun",
    };
    let month = match date.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    format!("{weekday} {month} {}", date.day())
}

/// Compact duration label for one Activity span: "<1m", "45m", "2h 05m".
fn duration_label(started_at_ms: i64, ended_at_ms: i64) -> String {
    let minutes = (ended_at_ms.saturating_sub(started_at_ms)).max(0) / 60_000;
    if minutes < 1 {
        "<1m".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    }
}

/// Local wall-clock `HH:MM` of `at_ms`, using the offset recovered from the
/// range start.
fn clock_label(at_ms: i64, local_offset_ms: i64) -> String {
    let minutes = at_ms.saturating_add(local_offset_ms).rem_euclid(DAY_MS) / 60_000;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Local `HH:MM–HH:MM` span of one Activity.
fn clock_span(activity: &Activity, local_offset_ms: i64) -> String {
    format!(
        "{}–{}",
        clock_label(activity.started_at_ms, local_offset_ms),
        clock_label(activity.ended_at_ms, local_offset_ms)
    )
}

/// Render one Activity as a single compact prompt line:
/// `- [Tue Jun 9 | 09:20–10:50 | 1h 30m | coding | deep | in Zed, Safari] Title — summary`
/// (category/focus/app segments are omitted when unset). `apps` are the
/// Activity's top evidence apps (`list_activity_meta_in_range`): app names only.
fn format_activity_line(activity: &Activity, local_offset_ms: i64, apps: &[String]) -> String {
    let mut meta = format!(
        "{} | {} | {}",
        day_label(activity.started_at_ms, local_offset_ms),
        clock_span(activity, local_offset_ms),
        duration_label(activity.started_at_ms, activity.ended_at_ms)
    );
    if let Some(category) = activity.category {
        meta.push_str(" | ");
        meta.push_str(category_label(category));
    }
    if let Some(focus) = activity.focus {
        meta.push_str(" | ");
        meta.push_str(focus_label(focus));
    }
    if !apps.is_empty() {
        meta.push_str(" | in ");
        meta.push_str(&apps.join(", "));
    }
    let title = activity.title.trim();
    let summary = truncate_chars(activity.summary.trim(), DIGEST_SUMMARY_CHAR_CAP);
    if summary.is_empty() {
        format!("- [{meta}] {title}\n")
    } else {
        format!("- [{meta}] {title} — {summary}\n")
    }
}

/// Render one Activity as the **compact** middle-tier line:
/// `- [Tue Jun 9 | 09:20–10:50 | 1h 30m] Title` — day + clock span + duration +
/// title only, dropping the summary, category, and focus the full
/// [`format_activity_line`] carries. Used for earlier episodes once the
/// full-detail prompt would blow the budget.
fn format_activity_line_compact(activity: &Activity, local_offset_ms: i64) -> String {
    format!(
        "- [{} | {} | {}] {}\n",
        day_label(activity.started_at_ms, local_offset_ms),
        clock_span(activity, local_offset_ms),
        duration_label(activity.started_at_ms, activity.ended_at_ms),
        activity.title.trim(),
    )
}

/// Render the **rollup** bottom-tier line for one local day's worth of the
/// oldest activities. `day_narrative` is the cached day-digest prose when one
/// exists for that local day (the **hybrid** path); when present its prose is
/// used verbatim and no stats are computed.
///
/// The **computed** fallback names: the activity count, the summed duration
/// (via [`duration_label`] over `0..total_ms`), the top
/// [`ROLLUP_TOP_CATEGORIES`] categories by summed duration (omitted when no
/// activity in the day carries a category), and up to [`ROLLUP_SAMPLE_TITLES`]
/// sample titles — the longest-duration episodes, short-capped at
/// [`ROLLUP_TITLE_CHAR_CAP`] (omitted when none have a non-empty title). Both
/// optional clauses collapse cleanly so a label-free day reads as just
/// `- [Tue Jun 9] 3 activities, 2h 10m`.
fn format_rollup_line(
    day_activities: &[&Activity],
    local_offset_ms: i64,
    day_narrative: Option<&str>,
) -> String {
    // The day label is taken from the first (chronologically earliest)
    // activity; they all share the same local day by construction.
    let label = day_activities
        .first()
        .map(|a| day_label(a.started_at_ms, local_offset_ms))
        .unwrap_or_else(|| day_label(0, local_offset_ms));

    // Hybrid path: reuse the day digest's own prose verbatim.
    if let Some(narrative) = day_narrative {
        let narrative = narrative.trim();
        if !narrative.is_empty() {
            return format!("- [{label}] {narrative}\n");
        }
    }

    // Computed path: count + summed duration + top categories + sample titles.
    let count = day_activities.len();
    let total_ms: i64 = day_activities
        .iter()
        .map(|a| a.ended_at_ms.saturating_sub(a.started_at_ms).max(0))
        .sum();
    let plural = if count == 1 { "activity" } else { "activities" };
    let mut line = format!(
        "- [{label}] {count} {plural}, {}",
        duration_label(0, total_ms)
    );

    // Top categories by summed duration. A stable insertion order keeps ties
    // deterministic (no `Date::now`/random tie-break).
    let mut category_ms: Vec<(ActivityCategory, i64)> = Vec::new();
    for activity in day_activities {
        let Some(category) = activity.category else {
            continue;
        };
        let span = activity.ended_at_ms.saturating_sub(activity.started_at_ms).max(0);
        match category_ms.iter_mut().find(|(c, _)| *c == category) {
            Some((_, ms)) => *ms = ms.saturating_add(span),
            None => category_ms.push((category, span)),
        }
    }
    if !category_ms.is_empty() {
        // Stable sort by descending duration preserves first-seen order on ties.
        category_ms.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = category_ms
            .iter()
            .take(ROLLUP_TOP_CATEGORIES)
            .map(|(category, ms)| format!("{} ({})", category_label(*category), duration_label(0, *ms)))
            .collect();
        line.push_str(" — top categories: ");
        line.push_str(&top.join(", "));
    }

    // Sample titles: the longest-duration episodes, short-capped.
    let mut by_duration: Vec<&&Activity> = day_activities.iter().collect();
    by_duration.sort_by(|a, b| {
        let da = a.ended_at_ms.saturating_sub(a.started_at_ms).max(0);
        let db = b.ended_at_ms.saturating_sub(b.started_at_ms).max(0);
        db.cmp(&da)
    });
    let samples: Vec<String> = by_duration
        .iter()
        .filter_map(|a| {
            let title = a.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(format!("\"{}\"", truncate_chars(title, ROLLUP_TITLE_CHAR_CAP)))
            }
        })
        .take(ROLLUP_SAMPLE_TITLES)
        .collect();
    if !samples.is_empty() {
        line.push_str("; e.g. ");
        line.push_str(&samples.join(", "));
    }

    line.push('\n');
    line
}

/// Detail tier each Activity is currently rendered at while degrading the
/// prompt to fit the budget. `Full` is the richest, `Rollup` the tersest.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tier {
    Full,
    Compact,
    Rollup,
}

/// Build the Digest prompt: a header naming the range, then the Activities in
/// chronological order (newest LAST — chronology reads better for a narrative).
///
/// When the full-detail lines would exceed [`DIGEST_PROMPT_CHAR_CAP`] the prompt
/// **degrades by recency** rather than dropping the oldest: the most recent
/// episodes stay [`Tier::Full`], earlier ones collapse to a one-line
/// [`Tier::Compact`] form, and the oldest are summarized one line per local day
/// ([`Tier::Rollup`]) — using a cached day-digest narrative from `day_digests`
/// when one exists for that day (the hybrid path), else computed stats. The
/// whole range is always represented, so the engine never mistakes a degraded
/// range for a partial one; a tier-explanation note says so when degradation
/// happened.
///
/// `day_digests` are DAY-kind cached digests overlapping the range; the slice is
/// empty for the "day" range kind (a day never reuses its own digest as input).
/// They are keyed by their `range_start_ms` — a local midnight in UTC ms — which
/// [`local_day_start_ms`] reproduces from an activity's `started_at_ms`.
fn build_digest_prompt(
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
    now_ms: i64,
    activities: &[Activity],
    day_digests: &[StoredDigest],
    apps: &HashMap<i64, Vec<String>>,
) -> String {
    let local_offset_ms = recover_local_offset_ms(range_start_ms);

    // Local-day-start (UTC ms) → cached day-digest narrative, for the hybrid
    // rollup path. Keyed exactly as `local_day_start_ms` computes a day key.
    let day_narratives: HashMap<i64, &str> = day_digests
        .iter()
        .map(|d| (d.range_start_ms, d.narrative.as_str()))
        .collect();

    // `activities` arrives chronological (oldest first) from the store query.
    // Index 0 is the oldest; degradation eats from the front.
    let n = activities.len();

    // Each activity starts at Full; we recompute the running total as we degrade.
    let full_lines: Vec<String> = activities
        .iter()
        .map(|a| format_activity_line(a, local_offset_ms, apps.get(&a.id).map_or(&[], |v| v)))
        .collect();
    let compact_lines: Vec<String> = activities
        .iter()
        .map(|a| format_activity_line_compact(a, local_offset_ms))
        .collect();
    let line_cost = |s: &str| s.chars().count();

    let mut tiers = vec![Tier::Full; n];
    let mut total: usize = full_lines.iter().map(|s| line_cost(s)).sum();
    let mut degraded = false;

    // The local-day key of each activity, used to group the rollup tier.
    let day_keys: Vec<i64> = activities
        .iter()
        .map(|a| local_day_start_ms(a.started_at_ms, local_offset_ms))
        .collect();

    // Step 1 (cap-fit): if everything fits at Full, emit unchanged. Else degrade.
    if total > DIGEST_PROMPT_CHAR_CAP {
        degraded = true;

        // Step 2: Full → Compact from oldest to newest, one at a time, stopping
        // as soon as we are under the cap (leaves newest=Full, oldest=Compact).
        for i in 0..n {
            if total <= DIGEST_PROMPT_CHAR_CAP {
                break;
            }
            tiers[i] = Tier::Compact;
            total = total + line_cost(&compact_lines[i]) - line_cost(&full_lines[i]);
        }

        // Step 3: still over after all-Compact → roll up whole oldest DAYS.
        // Group the currently-Compact prefix by local day; move the oldest such
        // day wholesale into a single Rollup line, recomputing, until under the
        // cap OR only one compact day remains. Full-tier activities are never
        // rolled up.
        if total > DIGEST_PROMPT_CHAR_CAP {
            // Build the rollup-line cost lazily per day as we collapse it.
            loop {
                // Indices still in Compact, contiguous from the oldest end.
                let compact_idx: Vec<usize> =
                    (0..n).filter(|&i| tiers[i] == Tier::Compact).collect();
                if compact_idx.is_empty() {
                    break;
                }
                // Distinct local days still represented in the Compact band.
                let mut compact_days: Vec<i64> = Vec::new();
                for &i in &compact_idx {
                    if !compact_days.contains(&day_keys[i]) {
                        compact_days.push(day_keys[i]);
                    }
                }
                if total <= DIGEST_PROMPT_CHAR_CAP || compact_days.len() <= 1 {
                    break;
                }

                // Oldest compact day = the first distinct day key (chronological).
                let target_day = compact_days[0];
                let members: Vec<&Activity> = (0..n)
                    .filter(|&i| tiers[i] == Tier::Compact && day_keys[i] == target_day)
                    .map(|i| &activities[i])
                    .collect();
                let rollup_line = format_rollup_line(
                    &members,
                    local_offset_ms,
                    day_narratives.get(&target_day).copied(),
                );
                // The rollup line replaces all that day's compact lines.
                let removed: usize = (0..n)
                    .filter(|&i| tiers[i] == Tier::Compact && day_keys[i] == target_day)
                    .map(|i| line_cost(&compact_lines[i]))
                    .sum();
                total = total - removed + line_cost(&rollup_line);
                for i in 0..n {
                    if tiers[i] == Tier::Compact && day_keys[i] == target_day {
                        tiers[i] = Tier::Rollup;
                    }
                }
            }
        }
    }

    // Materialize the lines in chronological order, collapsing each rollup DAY
    // to a single line at the position of its first (oldest) member.
    let mut body_lines: Vec<String> = Vec::new();
    let mut emitted_rollup_days: Vec<i64> = Vec::new();
    for i in 0..n {
        match tiers[i] {
            Tier::Full => body_lines.push(full_lines[i].clone()),
            Tier::Compact => body_lines.push(compact_lines[i].clone()),
            Tier::Rollup => {
                let day = day_keys[i];
                if emitted_rollup_days.contains(&day) {
                    continue;
                }
                emitted_rollup_days.push(day);
                let members: Vec<&Activity> = (0..n)
                    .filter(|&j| tiers[j] == Tier::Rollup && day_keys[j] == day)
                    .map(|j| &activities[j])
                    .collect();
                body_lines.push(format_rollup_line(
                    &members,
                    local_offset_ms,
                    day_narratives.get(&day).copied(),
                ));
            }
        }
    }

    // Step 4 (defensive, effectively unreachable): if even the fully-collapsed
    // body still exceeds the cap, drop the oldest body lines as a last resort
    // (the old behaviour) — bounded, never loops forever.
    let mut body_total: usize = body_lines.iter().map(|s| line_cost(s)).sum();
    while body_total > DIGEST_PROMPT_CHAR_CAP && body_lines.len() > 1 {
        body_total -= line_cost(&body_lines.remove(0));
        degraded = true;
    }

    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Below are the user's Activity episodes for one {range_kind}, in chronological order \
(oldest first). Each line carries the local day, the local start–end clock time, the episode \
duration, optional category and focus labels, the title, and a short summary. Write the 2-4 \
sentence second-person narrative digest, the headline, and the optional turn described in the \
instructions and return DigestNarrative.\n\n"
    ));
    prompt.push_str(&format!(
        "Range: {range_kind} [{range_start_ms} .. {range_end_ms}) ms ({} Activities)\n",
        activities.len()
    ));
    // Anchor the range in the user's LOCAL calendar (the per-line day labels are
    // already local) and state today, so the narrative speaks in the user's wall
    // time rather than the raw UTC millis above. `range_end_ms` is exclusive, so
    // the last covered day is `range_end_ms - 1`.
    prompt.push_str(&format!(
        "Local calendar: {} .. {} (the user's local time); today is {}. All day labels below are \
the user's local days.\n",
        day_label(range_start_ms, local_offset_ms),
        day_label(range_end_ms.saturating_sub(1), local_offset_ms),
        day_label(now_ms, local_offset_ms),
    ));
    prompt.push_str(&category_totals_line(activities));
    if degraded {
        prompt.push_str(
            "(Older activity is shown at reduced detail: the most recent episodes appear in full, \
earlier ones as brief one-line entries, and the oldest are summarized by local day. Nothing is \
missing — the Activity count above is the true total.)\n",
        );
    }
    prompt.push('\n');
    for line in &body_lines {
        prompt.push_str(line);
    }
    prompt
}

/// `Time by category: entertainment 41m (62%), research 18m (27%), …` over the
/// summed episode durations (overlapping episodes count twice; fine for a
/// shape). Without it the engine weighs lines equally, and a 10-second build in
/// an hour of videos read as "you focused on building".
fn category_totals_line(activities: &[Activity]) -> String {
    let mut totals: Vec<(&str, i64)> = Vec::new();
    for a in activities {
        let label = a.category.map_or("uncategorized", category_label);
        let span = a.ended_at_ms.saturating_sub(a.started_at_ms).max(0);
        match totals.iter_mut().find(|(l, _)| *l == label) {
            Some((_, ms)) => *ms += span,
            None => totals.push((label, span)),
        }
    }
    let sum: i64 = totals.iter().map(|(_, ms)| ms).sum();
    if sum == 0 {
        return String::new();
    }
    totals.sort_by(|a, b| b.1.cmp(&a.1));
    let parts: Vec<String> = totals
        .iter()
        .map(|(label, ms)| format!("{label} {} ({}%)", duration_label(0, *ms), ms * 100 / sum))
        .collect();
    format!("Time by category: {}\n", parts.join(", "))
}

/// Trim + validate an engine-returned narrative. `None` = generation failure
/// (the engine returned an empty/whitespace narrative).
fn normalize_narrative(raw: &str) -> Option<String> {
    let narrative = raw.trim();
    if narrative.is_empty() {
        None
    } else {
        Some(narrative.to_string())
    }
}

/// Char cap on the stored `headline — turn` string. An engine answer running
/// past this is truncated rather than rejected.
const HEADLINE_CHAR_CAP: usize = 140;

/// The separator the stored headline carries between headline and turn; the
/// lede greys everything after its FIRST occurrence.
const TURN_SEPARATOR: &str = " — ";

/// Join the engine's headline and optional turn into the one stored string
/// (`headline — turn`), clamped by [`normalize_headline`]. A dash the model put
/// in the headline anyway becomes a comma so the first [`TURN_SEPARATOR`] is
/// always the turn; a turn without a headline is dropped, and so is a turn that
/// would overrun the cap — a whole headline reads better than a clause cut short.
fn joined_headline(headline: &str, turn: &str) -> Option<String> {
    let headline = headline.trim().replace(TURN_SEPARATOR, ", ");
    let turn = turn.trim().trim_start_matches(['—', '–', '-']).trim_start();
    if turn.is_empty() || headline.is_empty() {
        return normalize_headline(&headline);
    }
    let headline = headline.trim_end_matches('.');
    let joined = format!("{headline}{TURN_SEPARATOR}{turn}");
    if joined.chars().count() > HEADLINE_CHAR_CAP {
        return normalize_headline(headline);
    }
    normalize_headline(&joined)
}

/// Trim + validate an engine-returned headline. Unlike [`normalize_narrative`],
/// `None` is NOT a failure — a narrative-only digest stays valid. An over-long
/// headline is cut at the last word boundary within [`HEADLINE_CHAR_CAP`]
/// chars (a hard char cut when one headline-sized word fills the cap) and ends
/// in "…" so the cut reads as one, not as a sentence that stopped;
/// `truncate_chars` is not reused because its mid-word ellipsis reads wrong in
/// the large display type the headline is set in.
fn normalize_headline(raw: &str) -> Option<String> {
    let headline = raw.trim();
    if headline.is_empty() {
        return None;
    }
    if headline.chars().count() <= HEADLINE_CHAR_CAP {
        return Some(headline.to_string());
    }
    let capped: String = headline.chars().take(HEADLINE_CHAR_CAP - 1).collect();
    let cut = match capped.rfind(char::is_whitespace) {
        Some(at) if at > 0 => capped[..at].trim_end(),
        _ => capped.as_str(),
    };
    Some(format!("{cut}…"))
}

/// Map a [`app_infra::StoredDigest`]-shaped row onto the wire DTO.
fn digest_dto(
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
    narrative: String,
    headline: Option<String>,
    generated_at_ms: i64,
) -> UserContextDigest {
    UserContextDigest {
        range_kind: range_kind.to_string(),
        range_start_ms,
        range_end_ms,
        narrative,
        headline,
        generated_at_ms,
    }
}

/// A served digest.
fn present(digest: UserContextDigest) -> UserContextDigestResult {
    UserContextDigestResult {
        digest: Some(digest),
        reason: None,
    }
}

/// No digest, and why (the non-error cases the Insights empty states tell apart).
fn absent(reason: DigestAbsentReason) -> UserContextDigestResult {
    UserContextDigestResult {
        digest: None,
        reason: Some(reason),
    }
}

/// Stamp one `derivation_run` ledger row for a Digest engine call (kind
/// `'digest'`). Token-usage bookkeeping deliberately reuses the existing run
/// ledger — that is what feeds `token_usage_totals` and the settings tokens-used
/// readout — but with **NULL window bounds**: `latest_derivation_run_window`
/// derives the forward Activity cursor from ANY run with non-NULL bounds, and a
/// digest range can end in the local FUTURE (today's end-of-day), which would
/// skip the worker's cursor past real captures. NULL bounds keep digest runs
/// invisible to the activity/backfill cursors (which also filter on kind),
/// matching how `'conclusion'`/`'confidence'` runs are recorded.
async fn record_digest_run(
    store: &UserContextStore,
    settings: &AiRuntimeSettings,
    status: &str,
    input_tokens: i64,
    output_tokens: i64,
    error: Option<String>,
) {
    let _ = store
        .insert_derivation_run(NewDerivationRun {
            kind: "digest".to_string(),
            window_start_ms: None,
            window_end_ms: None,
            status: status.to_string(),
            activities_derived: 0,
            conclusions_derived: 0,
            input_tokens,
            output_tokens,
            provider: provider_label_for(settings),
            model: model_label_for(settings),
            error,
            gate_drops: app_infra::DistillationGateDrops::default(),
        })
        .await;
}

/// Return the **User Context Digest** for one Insights Overview range,
/// generating (and caching) it when the range's Activity set changed.
///
/// A digest-less `Ok` carries its [`DigestAbsentReason`] (never an error): the
/// User Context opt-in off (`derivation_off`), the engine not configured
/// (`engine_not_ready`), or fewer than [`MIN_DIGEST_ACTIVITIES`] Activities in
/// range (`too_few`). `Err` covers real failures (a
/// malformed request, a store error, an engine that did not resolve live, an
/// engine call that failed / returned an empty narrative) and the
/// [`DIGEST_SENSITIVE_HOLD`] code. Engine failures and the hold are replayed to
/// the lazy path for [`FAILURE_COOLDOWN_MS`] on an unchanged input set — except
/// sign-in, rejected-key and settings failures, which the user fixes in Settings.
///
/// `user_context_enabled` is User Context's own continuous-derivation opt-in:
/// the digest is part of the User Context feature, so it honours the same opt-in
/// as the worker / run-now / status gates (a configured engine alone is not
/// enough — the user must have turned User Context on).
///
/// `force` is the manual **re-read** path (the Overview's re-digest button): it
/// skips the fingerprint cache hit AND the freshness floor so a stored narrative
/// is always regenerated from the range's current Activities, re-billing the
/// engine for one round trip. The opt-in / engine / minimum-Activity gates still
/// apply (a re-read cannot conjure a digest the normal path could never make);
/// only the staleness short-circuit is bypassed.
pub async fn get_or_generate_digest(
    ai_runtime: &AiRuntimeSettings,
    user_context_enabled: bool,
    store: &UserContextStore,
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
    force: bool,
) -> Result<UserContextDigestResult, String> {
    // A bad range is a frontend bug, not a normal-off case.
    if !matches!(range_kind, "day" | "week" | "month") {
        return Err(format!("unknown digest range kind: {range_kind}"));
    }
    if range_end_ms <= range_start_ms {
        return Err("digest range must be non-empty (endMs > startMs)".to_string());
    }

    // 1. User Context off / engine off / unresolved → silently no lede (same
    //    two-layer gate as the other user-context commands: the opt-in, then the
    //    shared engine-configured prerequisite).
    if !user_context_enabled {
        return Ok(absent(DigestAbsentReason::DerivationOff));
    }
    if let Err(reason) = crate::ai_runtime::engine_configured_prerequisite(ai_runtime).await {
        crate::native_capture::debug_log::log(format!(
            "digest: skipped {range_kind} (engine not ready: {reason})"
        ));
        return Ok(absent(DigestAbsentReason::EngineNotReady));
    }

    // 2./3. The range's Activities; a narrative over fewer than two is silly.
    //
    // LOAD-BEARING — DO NOT REMOVE. Activities are persisted UNFILTERED at
    // derivation (only Conclusions are gated there), so their title/summary may
    // carry sensitive text. The Digest prompt embeds raw activity title/summary
    // (`format_activity_line`) and ships it to a possibly-cloud engine. This
    // re-filter is the ONLY thing stopping a sensitive Activity from being
    // serialized into the digest prompt and egressing to a cloud engine —
    // mirroring `select_relevant_activities` in `brokered_access.rs` and the
    // `sensitive_activity_never_egresses_via_recall_context` regression test.
    // The output post-filter at step 5b only catches what the ENGINE writes; it
    // runs after the sensitive INPUT has already left the device. Filtering here
    // (before the count gate and fingerprint) keeps cache + count decisions in
    // step with what is actually sent.
    let activities: Vec<Activity> = store
        .list_activities_in_range(range_start_ms, range_end_ms)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|a| !guardrail::is_sensitive(&a.title, &a.summary))
        .collect();
    if activities.len() < MIN_DIGEST_ACTIVITIES {
        crate::native_capture::debug_log::log(format!(
            "digest: skipped {range_kind} (only {} activities, need {MIN_DIGEST_ACTIVITIES})",
            activities.len()
        ));
        return Ok(absent(DigestAbsentReason::TooFew));
    }

    // 4. Cache hit (the common path): an unchanged input set never re-bills the
    //    engine, and a changed one regenerates only past the per-range
    //    freshness floor — a current range churns on every worker beat, so a
    //    recent digest is served slightly stale instead of re-billing per
    //    visit. NOTE a racing second generation is fine — see the module doc.
    //    A forced re-read skips this short-circuit entirely and always
    //    regenerates from the range's current Activities.
    let fingerprint = digest_input_fingerprint(&activities);
    if !force {
        if let Some(stored) = store
            .get_digest(range_kind, range_start_ms)
            .await
            .map_err(|error| error.to_string())?
        {
            // A USER CORRECTION must win over the freshness floor (PR #112 digest
            // edge). The floor only exists to rate-limit *churn*-driven regeneration
            // (new captures arriving on a current range) — it must not pin a
            // narrative the user explicitly contradicted by relabeling an in-range
            // Activity, which would leave the corrected-away label visible in the
            // digest for up to the floor window (24h for a month). When an in-range
            // Activity was corrected AFTER this digest was generated, bypass the
            // floor and fall through to regenerate.
            let corrected_since_generated =
                range_has_correction_after(store, &activities, stored.generated_at_ms).await;

            if stored.input_fingerprint == fingerprint
                || (!corrected_since_generated
                    && within_freshness_floor(range_kind, stored.generated_at_ms, now_ms()))
            {
                return Ok(present(digest_dto(
                    &stored.range_kind,
                    stored.range_start_ms,
                    stored.range_end_ms,
                    stored.narrative,
                    stored.headline,
                    stored.generated_at_ms,
                )));
            }
        }
    }

    // 4b. Failure cooldown: a generation that failed (or was held back by the
    //     sensitive filter) on this same input set within the window is replayed
    //     to the lazy path without calling the model. Re-read bypasses it.
    if let Some(message) =
        cooldown_replay(range_kind, range_start_ms, &fingerprint, force, now_ms())
    {
        return Err(message);
    }
    let fail = |message: String| {
        cooldown_fail(range_kind, range_start_ms, &fingerprint, now_ms(), message)
    };

    // 5. Generate: guardrail-framed preamble + compact chronological prompt,
    //    structured extraction into the flat DigestNarrative shape. For week /
    //    month ranges, the oldest activities can roll up to one line per day,
    //    reusing any cached DAY-kind digest's narrative as that line's prose
    //    (the hybrid rollup path). A "day" range never reuses its own digest as
    //    input, so it passes no day digests.
    let day_digests = if range_kind == "day" {
        Vec::new()
    } else {
        store
            .list_day_digests_in_range(range_start_ms, range_end_ms)
            .await
            .map_err(|error| error.to_string())?
    };
    // ponytail: apps are enrichment, so a failed read just sends lines without them.
    let apps: HashMap<i64, Vec<String>> = store
        .list_activity_meta_in_range(range_start_ms, range_end_ms)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|meta| (meta.activity_id, meta.apps))
        .collect();
    let preamble = digest_preamble();
    let prompt = build_digest_prompt(
        range_kind,
        range_start_ms,
        range_end_ms,
        now_ms(),
        &activities,
        &day_digests,
        &apps,
    );
    let input_tokens = estimate_tokens(&preamble) + estimate_tokens(&prompt);

    crate::native_capture::debug_log::log(format!(
        "digest: generating {range_kind} [{range_start_ms}..{range_end_ms}) force={force} \
         activities={} day_digests={} prompt_chars={} est_input_tokens={input_tokens} \
         provider={:?} model={:?}",
        activities.len(),
        day_digests.len(),
        prompt.chars().count(),
        provider_label_for(ai_runtime),
        model_label_for(ai_runtime),
    ));

    // The LIVE engine resolve belongs here, not at the step-1 gate: for the
    // `chatgpt` kind it is a network round trip (an OAuth refresh against
    // auth.openai.com, 30s timeout, serialized behind the per-provider refresh
    // lock). Everything above this line — including the step-4 cache hit that
    // "never re-bills the engine" and is what the Insights surfaces hit on
    // every worker beat — needs no credential at all. Resolving it up front put
    // that round trip in front of the cache and let an offline refresh failure
    // withhold a digest already sitting on disk. Step 1's
    // `engine_configured_prerequisite` still gates on a *resolvable* engine
    // with no network.
    let engine = match crate::ai_runtime::resolve_engine_config_live(ai_runtime, None, None).await {
        Ok(engine) => engine,
        Err(reason) => {
            crate::native_capture::debug_log::log_warn(format!(
                "digest: skipped {range_kind} (engine config did not resolve: {reason})"
            ));
            // Both paths surface it (and record it) exactly like the extraction
            // failure below: a `too_few` absence renders as "not enough activity",
            // blaming the user's data for an engine failure. The cooldown keeps
            // the lazy path from re-trying on every Insights visit — except a
            // sign-in failure: re-resolving it needs no network (a rejected login
            // stops at step 1, a set with no refresh token fails locally), and a
            // cooldown would replay "sign in again" after the user signed back in.
            record_digest_run(
                store,
                ai_runtime,
                "failed",
                input_tokens,
                0,
                Some(reason.clone()),
            )
            .await;
            if reason.starts_with("needs_reconnect:") {
                return Err("The AI provider needs you to sign in again in Settings.".to_string());
            }
            return fail(
                "The AI engine is not reachable right now. Try again in a moment.".to_string(),
            );
        }
    };

    let extracted =
        ai_engine::extract_with_preamble::<DigestNarrative>(&engine, &preamble, &prompt).await;
    let batch = match extracted {
        Ok(batch) => batch,
        Err(error) => {
            // Log the failure kind plus the raw provider/transport error cut to
            // 200 chars (the user-facing message is a lossy one-liner; the raw
            // text's head carries the status code needed to tell "thinking mode
            // rejected tool_choice" apart from a bad key or an outage). Cut
            // because the body can echo prompt or output fragments, and user
            // logs leave the machine.
            crate::native_capture::debug_log::log_warn(format!(
                "digest: extraction FAILED for {range_kind} [{range_start_ms}..{range_end_ms}) ({}): {}",
                error.failure_kind(),
                truncate_chars(&error.to_string(), 200)
            ));
            // The ledger keeps the raw provider/transport detail for debugging;
            // the surface gets the one-sentence classification (rate limit,
            // rejected key, out of quota, unreachable, …) so a forced re-read can
            // tell the user WHY the inshort failed instead of silently omitting it.
            record_digest_run(
                store,
                ai_runtime,
                "failed",
                input_tokens,
                0,
                Some(error.to_string()),
            )
            .await;
            let message = error.user_facing_message_for(engine.cloud_provider());
            // A rejected key / bad setting is fixed in Settings and a retry isn't
            // billed, so it skips the cooldown — else the lazy read keeps
            // replaying the old failure after the fix.
            if matches!(error.failure_kind(), "auth" | "settings") {
                return Err(message);
            }
            return fail(message);
        }
    };
    let output_tokens = estimate_tokens(&batch.narrative)
        + estimate_tokens(&batch.headline)
        + estimate_tokens(&batch.turn);

    let Some(narrative) = normalize_narrative(&batch.narrative) else {
        record_digest_run(
            store,
            ai_runtime,
            "failed",
            input_tokens,
            output_tokens,
            Some("engine returned an empty narrative".to_string()),
        )
        .await;
        return fail("The AI engine returned an empty read. Try again in a moment.".to_string());
    };
    // An unusable headline is NOT a failure — the narrative-only digest stands.
    let headline = joined_headline(&batch.headline, &batch.turn);

    // 5b. HARD sensitive post-filter (PR #112 #8). The digest narrative is the
    //     engine's FREE TEXT, gated until now only by the SOFT guardrail preamble
    //     — and an LLM told to avoid a category will sometimes do it anyway. Run
    //     the same deterministic `is_sensitive` backstop used on Conclusions over
    //     the generated headline + narrative before anything is persisted or
    //     shown. A trip means the model wrote into an off-limits category: DROP
    //     the whole digest (do not persist, do not surface) and record a failed
    //     run, exactly as a sensitive Conclusion is dropped at derivation time.
    if guardrail::is_sensitive(headline.as_deref().unwrap_or(""), &narrative) {
        record_digest_run(
            store,
            ai_runtime,
            "failed",
            input_tokens,
            output_tokens,
            Some("digest narrative tripped the sensitive-category guardrail".to_string()),
        )
        .await;
        return fail(DIGEST_SENSITIVE_HOLD.to_string());
    }

    // 6. Persist, 7. record the run (see `record_digest_run` for why the run
    //    carries NULL window bounds), then return the fresh digest.
    let generated_at_ms = now_ms();
    store
        .upsert_digest(
            range_kind,
            range_start_ms,
            range_end_ms,
            &narrative,
            headline.as_deref(),
            &fingerprint,
            generated_at_ms,
        )
        .await
        .map_err(|error| error.to_string())?;
    record_digest_run(store, ai_runtime, "completed", input_tokens, output_tokens, None).await;
    cooldown_clear(range_kind, range_start_ms);

    crate::native_capture::debug_log::log_info(format!(
        "digest: generated {range_kind} [{range_start_ms}..{range_end_ms}) \
         narrative_chars={} output_tokens={output_tokens}",
        narrative.chars().count(),
    ));

    Ok(present(digest_dto(
        range_kind,
        range_start_ms,
        range_end_ms,
        narrative,
        headline,
        generated_at_ms,
    )))
}

#[cfg(test)]
#[path = "digest_tests.rs"]
mod tests;
