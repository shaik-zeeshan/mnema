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

mod prompt;
pub use prompt::*;

/// Base system instruction for the Digest pass. Like the Conclusion preamble,
/// the engine never sees this bare: [`digest_preamble`] prepends the soft
/// Sensitive Category Guardrail instruction first.
const DIGEST_PREAMBLE_BASE: &str = "You write a short narrative digest of a single user's derived \
Activity episodes over one calendar range (a day, a week, or a month). Write at most 2 short \
sentences, 40 words in total at most, of plain prose addressed to the user in the second person — \
for example \"Most of this week went into the billing migration, in long morning stretches \
in the editor.\". Say what took most of the time and one detail about how it went. What changed the \
shape of the range is the headline's turn, not the narrative's: do not restate it here. The page already shows the day-by-day breakdown, the stats, and every Activity, so do \
not recount the days or walk through episodes. Do NOT use markdown, lists, bullet points, headers, \
or quotes — only sentences. Ground every claim in the listed Activities; do not invent work that is \
not shown. Each Activity line carries its local clock times; mention one only when it is the point \
(\"after the late-morning call\"). Weigh the range by time: lead with what took the most of it (the time-by-category line \
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
        crate::debug_log::log(format!(
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
        crate::debug_log::log(format!(
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
                    // A shape-version bump (`vN:`) is not churn: never floor-serve old prose.
                    && stored.input_fingerprint.split(':').next() == fingerprint.split(':').next()
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

    crate::debug_log::log(format!(
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
            crate::debug_log::log_warn(format!(
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
            // Log the failure kind plus the error's head (the user-facing
            // message is a lossy one-liner; the head carries the status code
            // needed to tell "thinking mode rejected tool_choice" apart from a
            // bad key or an outage). `log_detail` drops the body and any quoted
            // model output: both echo captured content, and user logs leave the
            // machine.
            crate::debug_log::log_warn(format!(
                "digest: extraction FAILED for {range_kind} [{range_start_ms}..{range_end_ms}) ({}): {}",
                error.failure_kind(),
                error.log_detail()
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
    record_digest_run(
        store,
        ai_runtime,
        "completed",
        input_tokens,
        output_tokens,
        None,
    )
    .await;
    cooldown_clear(range_kind, range_start_ms);

    crate::debug_log::log_info(format!(
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
mod tests;

#[cfg(test)]
mod cache_tests;

#[cfg(test)]
mod narrative_tests;
