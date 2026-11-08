//! The command surface for **User Context** (issues #93/#94); each Tauri command
//! is a one-liner over these.
//!
//! These commands drive the "User Context" subsection of the Settings → Access
//! "Reasoning Engine" card: a status readout, read-only Activity + Conclusion
//! preview lists, a Subject view, and a manual "run derivation now" trigger (for
//! verification and the settings button). Dismiss / pin / wipe commands land
//! with their own slices (#97/#99) on this same module.

use std::sync::Arc;

use crate::recording_settings::{current_recording_settings, RecordingSettingsState};
use crate::{AppCore, AppInfraState, CoreEvent};
use capture_types::{
    Activity, ActivityCategory, ActivityMeta, AuthoredContext, Conclusion, DismissalState,
    DismissedView, FocusLevel, SubjectTrajectory, SubjectView, UserContextDigest,
    UserContextDigestResult, UserContextDistillationSummary, UserContextStatus,
    UserContextSummarizingFailure, UserContextTokenUsage,
};
use serde::Serialize;

use super::worker::{
    backfill_floor_ms, model_label_for, now_ms, provider_label_for, run_conclusion_distillation,
    run_forward_activity_window,
};

mod edits;
pub use edits::*;

/// Result of a manual one-window derivation pass (the "Derive latest now"
/// button). camelCase to match the rest of the wire DTOs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContextDerivationRunResult {
    pub activities_derived: i64,
    /// Conclusions upserted by the optional distillation pass that follows the
    /// Activity window (#94). 0 when distillation did not run / produced nothing.
    pub conclusions_derived: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub items_read: i64,
    pub message: String,
}

/// Availability + counts + token usage for the User Context settings surface.
///
/// `backfilling` (#98) reflects whether the background **History Backfill** still
/// has older history to cover — it drives the "building your understanding…"
/// progress line. It is the engine-available AND not-yet-at-the-floor condition;
/// see the inline computation. The floor is resolved from the SAME
/// `user_context` settings the worker uses (`backfill_floor_ms`).
pub async fn get_user_context_status(
    core: &AppCore,
    infra: &AppInfraState,
) -> Result<UserContextStatus, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>());
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Two-layer gate, mirroring the reason-code shape: continuous derivation is
    // "available" only when User Context's own opt-in is on AND the shared
    // engine-configured prerequisite is satisfied. The opt-in is checked first so
    // a user who configured the engine for Ask AI but not User Context sees the
    // distinct `user_context_disabled` reason rather than an engine reason.
    let (engine_available, reason) = if !user_context.enabled {
        (false, Some("user_context_disabled".to_string()))
    } else {
        match crate::ai_runtime::engine_configured_prerequisite(&ai_runtime).await {
            Ok(()) => (true, None),
            Err(reason) => (false, Some(reason)),
        }
    };

    let store = infra.user_context();
    let activity_count = store.count_activities().await.map_err(|e| e.to_string())?;
    let conclusion_count = store.count_conclusions().await.map_err(|e| e.to_string())?;
    let subject_count = store.count_subjects().await.map_err(|e| e.to_string())?;
    let dismissed_count = store.count_dismissed().await.map_err(|e| e.to_string())?;
    // Low-signal windows the derivation scheduler skipped before any LLM call,
    // over the last 24h (the debug surface's "distillation gate" readout).
    let skipped_windows_24h = store
        .count_skipped_runs_since(now_ms() - 24 * 60 * 60 * 1000)
        .await
        .map_err(|e| e.to_string())?;
    // Daily-digest freshness readout. Both best-effort — a read error degrades
    // to "unknown" rather than failing the whole status.
    let local_offset_minutes = store.local_offset_minutes().await.ok().flatten();
    let last_day_digest = store
        .latest_day_digest()
        .await
        .ok()
        .flatten()
        .map(|digest| UserContextDigest {
            range_kind: digest.range_kind,
            range_start_ms: digest.range_start_ms,
            range_end_ms: digest.range_end_ms,
            narrative: digest.narrative,
            headline: digest.headline,
            generated_at_ms: digest.generated_at_ms,
        });
    let last_derived_at_ms = store
        .last_derived_at_ms()
        .await
        .map_err(|e| e.to_string())?;
    // Best-effort: a read error degrades to "no known failure".
    let summarizing_failure = store
        .latest_window_failure()
        .await
        .ok()
        .flatten()
        .map(|failure| summarizing_failure_view(failure, &ai_runtime));
    // Summarized-up-to watermark: the end edge of the most-recently-COVERED
    // window (failed runs advanced the scheduler cursor but summarized nothing,
    // so they must not raise this), letting the frontend render the still-pending
    // region. Best-effort — a read error degrades to "unknown" rather than
    // failing the whole status.
    let covered_until_ms = store.covered_until_ms().await.ok().flatten();
    let token_usage = store
        .token_usage_totals()
        .await
        .map_err(|e| e.to_string())
        .unwrap_or(UserContextTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            run_count: 0,
        });

    // The most recent completed distillation pass with its per-gate withheld
    // counts — lets the readout explain a thin dossier instead of staying mute.
    let last_distillation = store
        .latest_distillation_summary()
        .await
        .ok()
        .flatten()
        .map(
            |(at_ms, conclusions_derived, drops)| UserContextDistillationSummary {
                at_ms,
                conclusions_derived,
                ungrounded: drops.ungrounded,
                guardrail_suppressed: drops.guardrail_suppressed,
                below_formation_bar: drops.below_formation_bar,
                resurface_blocked: drops.resurface_blocked,
            },
        );

    // --- backfilling progress (#98) ---
    // Cheap (two store reads): backfilling is true only when the engine is
    // available AND the background History Backfill still has older history to
    // reach (see `still_backfilling`). `earliest_capture` doubles as Overview's date-picker floor.
    let earliest_capture = store
        .earliest_capture_at_ms()
        .await
        .map_err(|e| e.to_string())?;
    let backfilling =
        engine_available && still_backfilling(store, &user_context, earliest_capture).await?;

    Ok(UserContextStatus {
        engine_available,
        reason,
        activity_count,
        conclusion_count,
        last_derived_at_ms,
        covered_until_ms,
        backfilling,
        token_usage,
        budget_tier: user_context.derivation_budget_tier,
        last_distillation,
        subject_count,
        dismissed_count,
        skipped_windows_24h,
        local_offset_minutes,
        last_day_digest,
        summarizing_failure,
        earliest_capture_at_ms: earliest_capture,
    })
}

/// Whether the background History Backfill still has older history to reach
/// (engine availability is the caller's gate). Two shapes count:
///   1. No windowed coverage exists yet but there ARE captures — the forward
///      pass has not even seeded coverage, so older history is pending.
///   2. Coverage exists but its oldest-covered edge is still above the floor,
///      AND there is older captured history below that edge to derive
///      (earliest_capture < oldest_covered). Without older captures there is
///      nothing to backfill even if the floor is lower.
/// The floor is resolved from the SAME settings the worker uses.
async fn still_backfilling(
    store: &app_infra::UserContextStore,
    user_context: &capture_types::UserContextSettings,
    earliest_capture: Option<i64>,
) -> Result<bool, String> {
    let oldest_covered = store
        .oldest_derivation_run_window_start()
        .await
        .map_err(|e| e.to_string())?;
    Ok(match oldest_covered {
        None => earliest_capture.is_some(),
        Some(oldest) => {
            let floor_ms = backfill_floor_ms(
                now_ms(),
                user_context.backfill_window_days,
                user_context.backfill_go_deeper,
                earliest_capture,
            );
            oldest > floor_ms && earliest_capture.map_or(false, |earliest| earliest < oldest)
        }
    })
}

/// Turn the store's failure streak into the readable status field: the
/// provider's display name plus one classified sentence (never a raw body).
fn summarizing_failure_view(
    failure: app_infra::user_context::WindowFailure,
    ai_runtime: &capture_types::AiRuntimeSettings,
) -> UserContextSummarizingFailure {
    use capture_types::AiProviderKind as Kind;
    let config = failure
        .provider
        .as_deref()
        .and_then(|id| ai_runtime.providers.iter().find(|p| p.id == id));
    let kind = config
        .map(|p| p.kind)
        .or_else(|| failure.provider.as_deref().and_then(Kind::from_id));
    let provider = match config.map(|p| p.label.trim()).filter(|l| !l.is_empty()) {
        Some(label) => Some(label.to_string()),
        None => kind.map(|kind| {
            match kind {
                Kind::Anthropic => "Anthropic",
                Kind::Openai => "OpenAI",
                Kind::OpenaiCompatible => "Your OpenAI-compatible provider",
                Kind::Chatgpt => "ChatGPT",
                Kind::Ollama => "Ollama",
                Kind::Llamafile => "Llamafile",
            }
            .to_string()
        }),
    };
    let cloud = matches!(kind, Some(Kind::Chatgpt)).then_some(ai_engine::CloudProvider::Chatgpt);
    UserContextSummarizingFailure {
        at_ms: failure.at_ms,
        failures: failure.failures_since_success,
        provider,
        reason: ai_engine::classify_provider_failure(failure.error.as_deref().unwrap_or(""), cloud),
    }
}

/// The most-recently-derived Activities (newest first) for the preview list.
///
/// When BOTH `start_ms` and `end_ms` are supplied, returns EVERY Activity
/// overlapping the half-open `[start_ms, end_ms)` window (oldest-first), so a
/// range-scoped view (e.g. Overview's selected day/week/month) gets the whole
/// period rather than a recency-capped slice — fixing the case where a busy
/// month exceeds the limit or a past range falls outside the newest page.
/// Range results ARE evidence-hydrated (the Journal's range read needs the
/// evidence refs); the Digest reads the lean, non-hydrated store method directly.
///
/// With no bounds it keeps the legacy `limit`/`offset` recency-paged behavior
/// (newest-first, evidence hydrated) for the preview / evidence-resolution
/// callers that depend on `activity.evidence`.
pub async fn list_user_context_activities(
    infra: &AppInfraState,
    limit: Option<i64>,
    offset: Option<i64>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Activity>, String> {
    if let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) {
        return infra
            .user_context()
            .list_activities_in_range_with_evidence(start_ms, end_ms)
            .await
            .map_err(|e| e.to_string());
    }
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let offset = offset.unwrap_or(0).max(0);
    infra
        .user_context()
        .list_recent_activities(limit, offset)
        .await
        .map_err(|e| e.to_string())
}

/// Apps (top 3 by evidence frames) and distinct speaker count for every Activity
/// overlapping `[start_ms, end_ms)` — the Journal cards' chips. Derived on read;
/// Activities with neither are absent (invoked as `{ startMs, endMs }`).
pub async fn list_user_context_activity_meta(
    infra: &AppInfraState,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<ActivityMeta>, String> {
    infra
        .user_context()
        .list_activity_meta_in_range(start_ms, end_ms)
        .await
        .map_err(|e| e.to_string())
}

/// Windows in `[start_ms, end_ms)` whose summarizing failed past the retry cap
/// (the Journal marks these holes at their real bounds). `[start, end]` pairs.
pub async fn list_failed_derivation_windows(
    infra: &AppInfraState,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<(i64, i64)>, String> {
    infra
        .user_context()
        .failed_windows_in_range(
            start_ms,
            end_ms,
            super::worker::WINDOW_RETRY_POLICY.max_attempts,
        )
        .await
        .map_err(|e| e.to_string())
}

/// The derived **Conclusion** dossier (highest-confidence first) for the preview
/// list. `visible` Conclusions always appear; `faded` (below the display floor)
/// are included only when `include_faded` is true; `dismissed` never appear.
pub async fn list_user_context_conclusions(
    infra: &AppInfraState,
    include_faded: Option<bool>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Conclusion>, String> {
    let include_faded = include_faded.unwrap_or(false);
    // Range-scoped (Overview feed) returns only Conclusions with a delta in the
    // window — bounded as the dossier grows. Absent bounds (Subjects, brokered
    // access) return the whole dossier, as before.
    if let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) {
        return infra
            .user_context()
            .list_conclusions_in_range(include_faded, start_ms, end_ms)
            .await
            .map_err(|e| e.to_string());
    }
    infra
        .user_context()
        .list_conclusions(include_faded)
        .await
        .map_err(|e| e.to_string())
}

/// The Subject page: every non-dismissed **Conclusion** about a Subject (faded
/// included) plus its confidence trajectories — the per-Conclusion
/// confidence-over-time lines drawn from **Confidence History** (#95). This is
/// the literal "warming up to a thing then cooling off" picture: each trajectory
/// is one Conclusion's snapshot series.
pub async fn get_user_context_subject(
    infra: &AppInfraState,
    subject: String,
) -> Result<SubjectView, String> {
    let store = infra.user_context();
    let conclusions = store
        .list_conclusions_for_subject(&subject)
        .await
        .map_err(|e| e.to_string())?;

    // One trajectory per Conclusion, built from its Confidence History snapshots.
    let mut trajectories: Vec<SubjectTrajectory> = Vec::with_capacity(conclusions.len());
    for conclusion in &conclusions {
        let history = store
            .list_confidence_history(conclusion.id)
            .await
            .map_err(|e| e.to_string())?;
        trajectories.push(SubjectTrajectory {
            conclusion_id: conclusion.id,
            statement: conclusion.statement.clone(),
            history,
        });
    }

    Ok(SubjectView {
        subject,
        conclusions,
        trajectories,
    })
}

/// Manually run ONE forward Activity-derivation window immediately (ignoring the
/// worker's tier pacing), then one Conclusion distillation pass over the
/// accumulated Activities. Used by the settings "Run derivation now" button and
/// for end-to-end verification. Returns a helpful message if the engine is
/// unavailable or there are no captures in range; never errors on those.
pub async fn user_context_run_derivation_now(
    core: &AppCore,
    infra: &AppInfraState,
) -> Result<UserContextDerivationRunResult, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>());
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Same two-layer gate as the worker: the User Context opt-in must be on, and
    // the shared engine-configured prerequisite must be satisfied. A manual run
    // honours the opt-in so it never derives behind a user who has not opted in.
    if !user_context.enabled {
        return Ok(UserContextDerivationRunResult {
            activities_derived: 0,
            conclusions_derived: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            items_read: 0,
            message: "User Context is off. Turn it on to derive Activities.".to_string(),
        });
    }

    if let Err(reason) = crate::ai_runtime::engine_configured_prerequisite(&ai_runtime).await {
        return Ok(UserContextDerivationRunResult {
            activities_derived: 0,
            conclusions_derived: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            items_read: 0,
            message: if reason == "ai_runtime_disabled" {
                "The Reasoning Engine is off. Enable it to derive Activities.".to_string()
            } else {
                format!("The Reasoning Engine is not ready ({reason}).")
            },
        });
    }

    // Background derivation always runs on the global default model (no pin,
    // no feature override).
    let engine = match crate::ai_runtime::resolve_engine_config_live(&ai_runtime, None, None).await
    {
        Ok(engine) => engine,
        Err(reason) => {
            return Ok(UserContextDerivationRunResult {
                activities_derived: 0,
                conclusions_derived: 0,
                window_start_ms: 0,
                window_end_ms: 0,
                items_read: 0,
                message: format!("The Reasoning Engine is not ready ({reason})."),
            });
        }
    };

    // Reuse the same forward-window path the worker runs, so manual and
    // automatic derivation behave identically.
    let infra = Arc::clone(infra);
    let provider_label = provider_label_for(&ai_runtime);
    let model_label = model_label_for(&ai_runtime);
    let run = run_forward_activity_window(
        &engine,
        infra.user_context(),
        provider_label.clone(),
        model_label.clone(),
    )
    .await;

    // Also run one distillation pass after the Activity window so a manual run
    // surfaces fresh Conclusions immediately (helpful for verification). It
    // shares the same engine and stamps its own `derivation_run` (kind
    // `'conclusion'`); distillation no-ops below two Activities.
    let distillation = run_conclusion_distillation(
        &engine,
        infra.user_context(),
        core,
        infra.subject_vectors(),
        provider_label,
        model_label,
    )
    .await;
    // "Conclusions upserted" = inserts AND updates. A distillation pass that
    // refreshes existing Conclusions on fresh evidence upserts via UPDATE (no new
    // row), so a count_conclusions before/after delta would report 0 while real
    // work happened. Use the outcome's own upserted count (also what the worker
    // stamps on its ledger row).
    let conclusions_derived = distillation.map_or(0, |outcome| outcome.upserted as i64);

    let distilled_changed = distillation.is_some_and(|outcome| outcome.upserted > 0);
    if run.changed || distilled_changed {
        core.host().emit(CoreEvent::UserContextChanged);
    }

    // Surface what distillation withheld so a manual run that "produced
    // nothing" tells the user why instead of looking like a silent no-op.
    let mut message = run.message;
    if let Some(summary) = distillation
        .map(|outcome| outcome.gate_drops)
        .filter(|drops| drops.total() > 0)
        .map(withheld_summary)
    {
        message.push(' ');
        message.push_str(&summary);
    }
    // The button derives only the latest window; say so while the background
    // History Backfill is still working through older captures. Best-effort.
    let store = infra.user_context();
    let earliest_capture = store.earliest_capture_at_ms().await.ok().flatten();
    if still_backfilling(store, &user_context, earliest_capture)
        .await
        .unwrap_or(false)
    {
        message.push_str(" Older history keeps filling in the background.");
    }

    Ok(UserContextDerivationRunResult {
        activities_derived: run.activities_derived,
        conclusions_derived,
        window_start_ms: run.window_start_ms,
        window_end_ms: run.window_end_ms,
        items_read: run.items_read,
        message,
    })
}

/// Plain-language sentence for the per-gate withheld counts of one distillation
/// pass, e.g. "Withheld 3 conclusion drafts: 1 by the privacy guardrail, 2
/// needing more evidence." Caller guarantees `drops.total() > 0`.
fn withheld_summary(drops: app_infra::DistillationGateDrops) -> String {
    let mut reasons: Vec<String> = Vec::new();
    if drops.guardrail_suppressed > 0 {
        reasons.push(format!(
            "{} by the privacy guardrail",
            drops.guardrail_suppressed
        ));
    }
    if drops.below_formation_bar > 0 {
        reasons.push(format!(
            "{} needing more evidence",
            drops.below_formation_bar
        ));
    }
    if drops.resurface_blocked > 0 {
        reasons.push(format!("{} honoring a dismissal", drops.resurface_blocked));
    }
    if drops.ungrounded > 0 {
        reasons.push(format!("{} without grounding", drops.ungrounded));
    }
    let total = drops.total();
    format!(
        "Withheld {total} conclusion draft{}: {}.",
        if total == 1 { "" } else { "s" },
        reasons.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::DismissalState;

    fn dismissal(subject: &str, statement: &str, dismissed_at_ms: i64) -> DismissalState {
        DismissalState {
            subject: subject.to_string(),
            statement: statement.to_string(),
            evidence_fingerprint: "fp".to_string(),
            evidence_activity_count: 1,
            dismissed_at_ms,
            source: "user".to_string(),
        }
    }

    #[test]
    fn authored_validation_limits_length_and_fixes_topics() {
        assert_eq!(validate_authored("  hi ", Some(" ")), Ok(("hi", None)));
        assert_eq!(
            validate_authored("hi", Some("Goals")),
            Ok(("hi", Some("Goals")))
        );
        assert!(validate_authored("hi", Some("role")).is_err());
        assert!(
            validate_authored("hi", Some("work")).is_err(),
            "exact names only"
        );
        assert!(validate_authored("   ", None).is_err());
        let at_limit = "é".repeat(AUTHORED_STATEMENT_CHAR_LIMIT);
        assert!(
            validate_authored(&at_limit, None).is_ok(),
            "chars, not bytes"
        );
        assert!(validate_authored(&format!("{at_limit}x"), None).is_err());
    }

    #[test]
    fn authored_row_round_trips_flat_with_in_prompt() {
        let row = AuthoredContextRow {
            item: AuthoredContext {
                id: 5,
                text: "I'm a designer".to_string(),
                topic: Some("Work".to_string()),
                created_at_ms: 10,
                updated_at_ms: 20,
            },
            in_prompt: false,
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": 5, "text": "I'm a designer", "topic": "Work",
                "createdAtMs": 10, "updatedAtMs": 20, "inPrompt": false
            })
        );
        assert_eq!(
            serde_json::from_value::<AuthoredContextRow>(value).unwrap(),
            row
        );
    }

    #[test]
    fn dedupe_dismissed_collapses_duplicates_keeping_newest() {
        // Newest-first global order, as `list_dismissals` returns it.
        let input = vec![
            dismissal("Apple", "Interested in Apple", 200),
            dismissal("Rust", "Learning Rust", 150),
            dismissal("Apple", "Interested in Apple", 100),
        ];

        let views = dedupe_dismissed(input);

        // One entry per (subject, statement); the Apple duplicate collapses to its
        // newest dismissal; newest-first order is preserved.
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].subject, "Apple");
        assert_eq!(views[0].dismissed_at_ms, 200);
        assert_eq!(views[1].subject, "Rust");
    }

    #[test]
    fn dedupe_dismissed_keys_case_insensitively() {
        let input = vec![
            dismissal("Apple", "Interested in Apple", 200),
            dismissal("apple", "INTERESTED IN APPLE", 100),
        ];

        let views = dedupe_dismissed(input);

        assert_eq!(views.len(), 1, "case variants are the same belief");
        assert_eq!(views[0].dismissed_at_ms, 200);
    }

    #[test]
    fn dedupe_dismissed_folds_only_ascii_case_to_match_undismiss_nocase() {
        // `undismiss` and the resurface gate match beliefs with SQLite
        // `COLLATE NOCASE`, which folds ONLY ASCII A–Z. Two dismissals differing
        // solely by a non-ASCII case (É vs é) are DISTINCT veto rows that
        // `undismiss(subject, statement)` deletes one casing at a time, so dedupe
        // must keep them separate — collapsing them leaves a stale, un-restorable
        // suppression after the single archive row is restored. (Regression: the
        // dedupe key previously used full-Unicode `to_lowercase`, over-folding.)
        let input = vec![
            dismissal("CAFÉ", "Likes CAFÉ", 200),
            dismissal("café", "Likes café", 100),
        ];

        let views = dedupe_dismissed(input);

        assert_eq!(
            views.len(),
            2,
            "non-ASCII case variants are distinct vetoes under NOCASE"
        );
    }

    #[test]
    fn summarizing_failure_names_the_provider_and_classifies_the_reason() {
        let mut settings = capture_types::AiRuntimeSettings::default();
        settings.providers.push(capture_types::AiProviderConfig {
            id: "anthropic".to_string(),
            kind: capture_types::AiProviderKind::Anthropic,
            label: String::new(),
            base_url: String::new(),
        });
        let view = summarizing_failure_view(
            app_infra::user_context::WindowFailure {
                at_ms: 5,
                provider: Some("anthropic".to_string()),
                error: Some(
                    "HTTP 429 Too Many Requests: {\"type\":\"rate_limit_error\"}".to_string(),
                ),
                failures_since_success: 6,
            },
            &settings,
        );
        assert_eq!(view.provider.as_deref(), Some("Anthropic"));
        assert_eq!(view.failures, 6);
        assert!(view.reason.contains("rate-limiting"), "{}", view.reason);
        assert!(!view.reason.contains('{'));
    }

    #[test]
    fn authored_edits_emit_user_context_changed_only_when_they_land() {
        let dir = tempfile::tempdir().expect("tempdir");
        let host = std::sync::Arc::new(crate::test_support::FakeHost::default());
        let core = crate::test_support::core_in(host.clone(), dir.path());
        core.block_on(async {
            let infra: AppInfraState = Arc::new(
                ::app_infra::AppInfra::initialize(&dir.path().join("data"))
                    .await
                    .expect("infra should initialize"),
            );

            assert!(
                user_context_add_authored(&core, &infra, "  ".to_string(), None)
                    .await
                    .is_err()
            );
            assert!(host.events.lock().unwrap().is_empty());

            let added = user_context_add_authored(
                &core,
                &infra,
                "Prefers async standups".to_string(),
                Some("Work".to_string()),
            )
            .await
            .expect("add");
            user_context_delete_authored(&core, &infra, added.id)
                .await
                .expect("delete");
        });
        assert_eq!(
            *host.events.lock().unwrap(),
            vec![CoreEvent::UserContextChanged, CoreEvent::UserContextChanged]
        );
    }
}
