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

use capture_types::{Activity, AiRuntimeSettings, FocusLevel, UserContextDigest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use app_infra::user_context::guardrail;
use app_infra::{digest_input_fingerprint, NewDerivationRun, UserContextStore};

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
Also write a short headline for the range: at most six words, sentence case, concrete and \
grounded in the dominant work — for example \"A deep week in the editor\". No clickbait, no \
terminal punctuation, no surrounding quotes. \
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

/// Per-activity summary cap inside the Digest prompt, so one verbose Activity
/// cannot dominate the budget. Mirrors `derivation.rs`'s per-item-cap approach
/// (`ACTIVITY_SUMMARY_CHAR_CAP` there is module-private, so the value is
/// restated here); kept smaller because a month range can hold many Activities.
const DIGEST_SUMMARY_CHAR_CAP: usize = 240;

/// Total char budget for the rendered Activity lines. Mirrors how
/// `derivation.rs` bounds its prompt inputs (per-item caps + a bounded item
/// count via `MAX_ITEMS`); expressed here as a char budget because the input is
/// a whole calendar range rather than a capture window. ~6k estimated tokens.
const DIGEST_PROMPT_CHAR_CAP: usize = 24_000;

/// The structured result the engine returns for one Digest pass. Flat (two
/// string fields), so the schema is `$ref`/`$defs`-free without `#[schemars(inline)]`;
/// the reference-free guard test below keeps it that way.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DigestNarrative {
    /// 2–4 sentences of second-person plain prose.
    pub narrative: String,
    /// Short headline above the prose (≤ ~6 words). Defaulted (same stance as
    /// `derivation.rs`'s optional fields) so an engine that omits it still
    /// yields a narrative-only digest instead of an extraction failure;
    /// [`normalize_headline`] turns empty/whitespace into `None`.
    #[serde(default)]
    pub headline: String,
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

/// Render one Activity as a single compact prompt line:
/// `- [Tue Jun 9 | 1h 30m | coding | deep] Title — summary`
/// (category/focus segments are omitted when unset).
fn format_activity_line(activity: &Activity, local_offset_ms: i64) -> String {
    let mut meta = format!(
        "{} | {}",
        day_label(activity.started_at_ms, local_offset_ms),
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
    let title = activity.title.trim();
    let summary = truncate_chars(activity.summary.trim(), DIGEST_SUMMARY_CHAR_CAP);
    if summary.is_empty() {
        format!("- [{meta}] {title}\n")
    } else {
        format!("- [{meta}] {title} — {summary}\n")
    }
}

/// Build the Digest prompt: a header naming the range, then one line per
/// Activity in chronological order (newest LAST — chronology reads better for a
/// narrative). When the rendered lines exceed [`DIGEST_PROMPT_CHAR_CAP`], the
/// OLDEST lines are dropped first (the same newest-matters-most stance the
/// derivation worker takes) and the omission is stated so the engine does not
/// mistake a truncated range for the whole story.
fn build_digest_prompt(
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
    activities: &[Activity],
) -> String {
    let local_offset_ms = recover_local_offset_ms(range_start_ms);

    // `activities` arrives chronological (oldest first) from the store query.
    let mut lines: Vec<String> = activities
        .iter()
        .map(|activity| format_activity_line(activity, local_offset_ms))
        .collect();

    // Enforce the total budget by dropping the OLDEST lines first.
    let mut total: usize = lines.iter().map(|line| line.chars().count()).sum();
    let mut omitted = 0usize;
    while total > DIGEST_PROMPT_CHAR_CAP && lines.len() > 1 {
        total -= lines.remove(0).chars().count();
        omitted += 1;
    }

    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Below are the user's Activity episodes for one {range_kind}, in chronological order \
(oldest first). Each line carries the local day, the episode duration, optional category and \
focus labels, the title, and a short summary. Write the 2-4 sentence second-person narrative \
digest and the short headline described in the instructions and return DigestNarrative.\n\n"
    ));
    prompt.push_str(&format!(
        "Range: {range_kind} [{range_start_ms} .. {range_end_ms}) ms ({} Activities)\n",
        activities.len()
    ));
    if omitted > 0 {
        prompt.push_str(&format!(
            "(The {omitted} oldest Activities were omitted for length; the count above is the \
true total.)\n"
        ));
    }
    prompt.push('\n');
    for line in &lines {
        prompt.push_str(line);
    }
    prompt
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

/// Headline char cap. The preamble asks for ≤ ~6 words; an engine answer
/// running past this is truncated rather than rejected.
const HEADLINE_CHAR_CAP: usize = 80;

/// Trim + validate an engine-returned headline. Unlike [`normalize_narrative`],
/// `None` is NOT a failure — a narrative-only digest stays valid. An over-long
/// headline is cut at the last word boundary within [`HEADLINE_CHAR_CAP`]
/// chars (a hard char cut when one headline-sized word fills the cap);
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
    let capped: String = headline.chars().take(HEADLINE_CHAR_CAP).collect();
    let cut = match capped.rfind(char::is_whitespace) {
        Some(at) if at > 0 => capped[..at].trim_end(),
        _ => capped.as_str(),
    };
    Some(cut.to_string())
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
/// `Ok(None)` is the silent-omission path (never an error): the User Context
/// opt-in off, the engine off / unresolved, or fewer than
/// [`MIN_DIGEST_ACTIVITIES`] Activities in range. `Err` is reserved for real
/// failures: a malformed request, a store error, or an engine call that failed /
/// returned an empty narrative.
///
/// `user_context_enabled` is User Context's own continuous-derivation opt-in:
/// the digest is part of the User Context feature, so it honours the same opt-in
/// as the worker / run-now / status gates (a configured engine alone is not
/// enough — the user must have turned User Context on).
pub async fn get_or_generate_digest(
    ai_runtime: &AiRuntimeSettings,
    user_context_enabled: bool,
    store: &UserContextStore,
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
) -> Result<Option<UserContextDigest>, String> {
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
        return Ok(None);
    }
    if crate::ai_runtime::engine_configured_prerequisite(ai_runtime)
        .await
        .is_err()
    {
        return Ok(None);
    }
    let Ok(engine) = crate::ai_runtime::resolve_engine_config(ai_runtime, None, None) else {
        return Ok(None);
    };

    // 2./3. The range's Activities; a narrative over fewer than two is silly.
    let activities = store
        .list_activities_in_range(range_start_ms, range_end_ms)
        .await
        .map_err(|error| error.to_string())?;
    if activities.len() < MIN_DIGEST_ACTIVITIES {
        return Ok(None);
    }

    // 4. Cache hit (the common path): an unchanged input set never re-bills the
    //    engine, and a changed one regenerates only past the per-range
    //    freshness floor — a current range churns on every worker beat, so a
    //    recent digest is served slightly stale instead of re-billing per
    //    visit. NOTE a racing second generation is fine — see the module doc.
    let fingerprint = digest_input_fingerprint(&activities);
    if let Some(stored) = store
        .get_digest(range_kind, range_start_ms)
        .await
        .map_err(|error| error.to_string())?
    {
        if stored.input_fingerprint == fingerprint
            || within_freshness_floor(range_kind, stored.generated_at_ms, now_ms())
        {
            return Ok(Some(digest_dto(
                &stored.range_kind,
                stored.range_start_ms,
                stored.range_end_ms,
                stored.narrative,
                stored.headline,
                stored.generated_at_ms,
            )));
        }
    }

    // 5. Generate: guardrail-framed preamble + compact chronological prompt,
    //    structured extraction into the flat DigestNarrative shape.
    let preamble = digest_preamble();
    let prompt = build_digest_prompt(range_kind, range_start_ms, range_end_ms, &activities);
    let input_tokens = estimate_tokens(&preamble) + estimate_tokens(&prompt);

    let extracted =
        ai_engine::extract_with_preamble::<DigestNarrative>(&engine, &preamble, &prompt).await;
    let batch = match extracted {
        Ok(batch) => batch,
        Err(error) => {
            let message = error.to_string();
            record_digest_run(store, ai_runtime, "failed", input_tokens, 0, Some(message.clone()))
                .await;
            return Err(format!("Digest generation failed: {message}"));
        }
    };
    let output_tokens = estimate_tokens(&batch.narrative) + estimate_tokens(&batch.headline);

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
        return Err("Digest generation failed: engine returned an empty narrative".to_string());
    };
    // An unusable headline is NOT a failure — the narrative-only digest stands.
    let headline = normalize_headline(&batch.headline);

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

    Ok(Some(digest_dto(
        range_kind,
        range_start_ms,
        range_end_ms,
        narrative,
        headline,
        generated_at_ms,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::ActivityCategory;

    fn activity(
        id: i64,
        started_at_ms: i64,
        ended_at_ms: i64,
        title: &str,
        summary: &str,
        category: Option<ActivityCategory>,
        focus: Option<FocusLevel>,
    ) -> Activity {
        Activity {
            id,
            title: title.to_string(),
            summary: summary.to_string(),
            category,
            focus,
            started_at_ms,
            ended_at_ms,
            created_at_ms: started_at_ms,
            evidence: Vec::new(),
        }
    }

    /// Same guard as `derivation.rs`: the engine schema must stay
    /// `$ref`/`$defs`-free for reference-less structured-output backends.
    #[test]
    fn digest_schema_is_reference_free() {
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
        let schema = serde_json::to_value(schemars::schema_for!(DigestNarrative)).unwrap();
        assert!(!has_reference(&schema), "schema has a reference: {schema}");
    }

    #[test]
    fn digest_preamble_leads_with_the_guardrail_and_bans_markdown() {
        let preamble = digest_preamble();
        assert!(preamble.starts_with(guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION));
        assert!(preamble.contains("second person"));
        assert!(preamble.contains("Do NOT use markdown"));
        // The headline ask rides the same preamble: short, grounded, unadorned.
        assert!(preamble.contains("short headline"));
        assert!(preamble.contains("at most six words"));
        assert!(preamble.contains("no terminal punctuation, no surrounding quotes"));
    }

    #[test]
    fn recovers_local_offset_from_local_midnight_starts() {
        // 2026-06-08 00:00 in each zone, expressed as a UTC unix-ms instant.
        let utc_midnight = 1_780_876_800_000_i64; // 20612 days, divisible by DAY_MS
        assert_eq!(utc_midnight % DAY_MS, 0, "fixture must be a UTC midnight");
        // UTC: midnight is on the day boundary → offset 0.
        assert_eq!(recover_local_offset_ms(utc_midnight), 0);
        // UTC+5:30 (IST): local midnight is 18:30 UTC the previous day.
        let ist = utc_midnight - (5 * 60 + 30) * 60_000;
        assert_eq!(recover_local_offset_ms(ist), (5 * 60 + 30) * 60_000);
        // UTC−8 (PST): local midnight is 08:00 UTC the same day.
        let pst = utc_midnight + 8 * 3_600_000;
        assert_eq!(recover_local_offset_ms(pst), -8 * 3_600_000);
        // UTC+13 (Tonga): preferred over the ambiguous UTC−11 alias.
        let tonga = utc_midnight - 13 * 3_600_000;
        assert_eq!(recover_local_offset_ms(tonga), 13 * 3_600_000);
    }

    #[test]
    fn day_label_renders_the_local_weekday_and_date() {
        // 2026-06-08 is a Monday. 10:00 local in UTC−8 = 18:00 UTC.
        let utc_midnight = 1_780_876_800_000_i64;
        let local_offset_ms = -8 * 3_600_000;
        let at = utc_midnight + 18 * 3_600_000;
        assert_eq!(day_label(at, local_offset_ms), "Mon Jun 8");
        // The same instant labeled in UTC falls on the same calendar day here,
        // but a late-evening local time crosses: 23:00 UTC−8 = 07:00 UTC Tue.
        let late = utc_midnight + 31 * 3_600_000;
        assert_eq!(day_label(late, local_offset_ms), "Mon Jun 8");
        assert_eq!(day_label(late, 0), "Tue Jun 9");
    }

    #[test]
    fn duration_label_is_compact() {
        assert_eq!(duration_label(0, 30_000), "<1m");
        assert_eq!(duration_label(0, 45 * 60_000), "45m");
        assert_eq!(duration_label(0, 125 * 60_000), "2h 05m");
        // A clamped/garbage negative span never panics.
        assert_eq!(duration_label(1_000, 0), "<1m");
    }

    #[test]
    fn activity_line_carries_day_duration_labels_and_omits_unset() {
        let labeled = activity(
            1,
            0,
            90 * 60_000,
            "Billing migration",
            "Moved invoices to the new schema.",
            Some(ActivityCategory::Creating),
            Some(FocusLevel::Deep),
        );
        let line = format_activity_line(&labeled, 0);
        assert!(line.contains("Thu Jan 1"), "epoch day label: {line}");
        assert!(line.contains("1h 30m"));
        assert!(line.contains("| creating | deep]"));
        assert!(line.contains("Billing migration — Moved invoices"));

        // Unset category/focus and empty summary leave no dangling separators.
        let bare = activity(2, 0, 60_000, "Misc", "  ", None, None);
        let line = format_activity_line(&bare, 0);
        assert!(!line.contains("| creating"));
        assert!(!line.contains("—"));
        assert!(line.trim_end().ends_with("Misc"));
    }

    #[test]
    fn prompt_is_chronological_with_a_range_header() {
        let activities = vec![
            activity(1, 1_000, 61_000, "Oldest", "first", None, None),
            activity(2, 100_000, 160_000, "Newest", "last", None, None),
        ];
        let prompt = build_digest_prompt("week", 0, DAY_MS * 7, &activities);
        assert!(prompt.contains("Range: week [0 .. 604800000) ms (2 Activities)"));
        assert!(prompt.contains("one week"));
        assert!(prompt.contains("the short headline"), "prompt asks for the headline");
        let oldest = prompt.find("Oldest").expect("oldest line");
        let newest = prompt.find("Newest").expect("newest line");
        assert!(oldest < newest, "chronological: oldest first, newest last");
        assert!(!prompt.contains("omitted"), "no omission note under budget");
    }

    #[test]
    fn prompt_drops_oldest_lines_over_the_char_cap_and_says_so() {
        let long_summary = "x".repeat(DIGEST_SUMMARY_CHAR_CAP);
        let activities: Vec<Activity> = (0..200)
            .map(|i| {
                activity(
                    i,
                    i * 60_000,
                    (i + 1) * 60_000,
                    &format!("Activity {i}"),
                    &long_summary,
                    None,
                    None,
                )
            })
            .collect();
        let prompt = build_digest_prompt("month", 0, DAY_MS * 30, &activities);
        // Bounded: the body stays near the cap (header + one-line slack).
        assert!(
            prompt.chars().count() < DIGEST_PROMPT_CHAR_CAP + 800,
            "prompt stays bounded: {} chars",
            prompt.chars().count()
        );
        // Newest survives, oldest is dropped, and the omission is stated with
        // the true total.
        assert!(prompt.contains("Activity 199"));
        assert!(!prompt.contains("[Thu Jan 1 | 1m] Activity 0 "));
        assert!(prompt.contains("oldest Activities were omitted"));
        assert!(prompt.contains("(200 Activities)"));
    }

    /// The freshness floor widens with the range: day 1h, week 6h, month 24h.
    /// A digest inside its floor is served even on fingerprint mismatch; one
    /// past it regenerates. A backwards clock (negative age) counts as fresh.
    #[test]
    fn freshness_floor_widens_with_range_and_gates_on_age() {
        const HOUR_MS: i64 = 60 * 60 * 1000;
        assert_eq!(freshness_floor_ms("day"), HOUR_MS);
        assert_eq!(freshness_floor_ms("week"), 6 * HOUR_MS);
        assert_eq!(freshness_floor_ms("month"), 24 * HOUR_MS);

        let generated_at = 1_000_000_000_000_i64;
        // Just inside vs. at the day floor.
        assert!(within_freshness_floor("day", generated_at, generated_at + HOUR_MS - 1));
        assert!(!within_freshness_floor("day", generated_at, generated_at + HOUR_MS));
        // A week-old month digest is still fresh; a day-old week digest is not.
        assert!(within_freshness_floor("month", generated_at, generated_at + 23 * HOUR_MS));
        assert!(!within_freshness_floor("week", generated_at, generated_at + 24 * HOUR_MS));
        // Clock moved backwards: fresh, never a forced regeneration.
        assert!(within_freshness_floor("day", generated_at, generated_at - HOUR_MS));
    }

    #[test]
    fn normalize_narrative_trims_and_rejects_empty() {
        assert_eq!(
            normalize_narrative("  A focused week. \n"),
            Some("A focused week.".to_string())
        );
        assert_eq!(normalize_narrative("   \n\t"), None);
        assert_eq!(normalize_narrative(""), None);
    }

    /// Headline post-validation: trim; empty/whitespace → `None` (NOT a
    /// failure); over-long → word-boundary truncation under
    /// [`HEADLINE_CHAR_CAP`]; a single cap-filling word → hard char cut.
    #[test]
    fn normalize_headline_trims_drops_empty_and_truncates_on_word_boundaries() {
        assert_eq!(
            normalize_headline("  A deep week in the editor \n"),
            Some("A deep week in the editor".to_string())
        );
        assert_eq!(normalize_headline("   \n\t"), None);
        assert_eq!(normalize_headline(""), None);

        // At the cap exactly: untouched.
        let exact = "x".repeat(HEADLINE_CHAR_CAP);
        assert_eq!(normalize_headline(&exact), Some(exact.clone()));

        // Over the cap: cut at the LAST word boundary inside the cap, with no
        // dangling whitespace and no mid-word fragment.
        let long = format!("{} tail", "word ".repeat(20).trim_end()); // 104 chars
        let truncated = normalize_headline(&long).expect("headline");
        assert!(truncated.chars().count() <= HEADLINE_CHAR_CAP);
        assert_eq!(truncated, "word ".repeat(16).trim_end());

        // One unbroken cap-filling word: hard char cut, never `None`.
        let unbroken = "y".repeat(HEADLINE_CHAR_CAP + 20);
        assert_eq!(normalize_headline(&unbroken), Some("y".repeat(HEADLINE_CHAR_CAP)));
    }
}
