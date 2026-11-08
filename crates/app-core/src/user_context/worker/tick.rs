use super::*;

/// Conclusion distillation runs at most once per this interval, and only when
/// an Activity was inserted or extended since the last successful pass, so the
/// dossier (expensive, occasional) re-distills far less often than the diary
/// (cheap, frequent). Each pass sends only what changed (`distill_conclusions`).
/// The manual run-now command bypasses this throttle.
pub(super) const CONCLUSION_MIN_INTERVAL_MS: i64 = 30 * 60 * 1000;

/// Run the confidence-decay beat once every this many ticks. This is the
/// *slowest* beat: confidence fades on a 30-day
/// half-life, so re-evaluating decay only every few ticks is plenty, and decay
/// uses **no LLM** (it is pure local math). Counting ticks (not successful
/// Activity ticks) means the dossier keeps fading during quiet stretches even
/// when no new Activities are arriving — silence is exactly when decay matters.
pub(super) const CONFIDENCE_DECAY_EVERY_M_TICKS: u32 = 6;

/// Keep at most this many **Confidence History** snapshots per Conclusion. History
/// is aggressively prunable: recency-weighting means old snapshots stop mattering,
/// so the trajectory keeps only a recent tail. At one snapshot per decay beat this
/// is many days of arc, far more than the Subject line needs.
pub(super) const MAX_SNAPSHOTS_PER_CONCLUSION: i64 = 64;

/// Cross-tick state for the slower Conclusion-distillation beat. The worker loop
/// owns one of these and threads it through each tick. Activity derivation stays
/// the frequent beat; distillation is throttled to [`CONCLUSION_MIN_INTERVAL_MS`].
#[derive(Debug, Default)]
pub struct WorkerCadence {
    /// When the worker last attempted a distillation pass (failed ones too, so a
    /// failing engine is not re-billed every tick). Seeded at worker start from
    /// the last completed pass in the ledger, so a restart does not re-distill.
    pub last_distillation_at_ms: Option<i64>,
    /// The last pass succeeded but more changed Activities wait behind the
    /// watermark than one pass sends (e.g. a History Backfill), so the next tick
    /// distills again without waiting out [`CONCLUSION_MIN_INTERVAL_MS`].
    pub distillation_backlog: bool,
    /// Ticks (of any kind, including idle-but-resolved ones that reach the
    /// decay check) since the last confidence-decay pass.
    pub ticks_since_decay: u32,
}

impl WorkerCadence {
    /// Whether this tick should run a Conclusion distillation: only when an
    /// Activity was inserted or extended since the last successful pass, and —
    /// unless a backlog is draining — at most once per [`CONCLUSION_MIN_INTERVAL_MS`].
    pub(super) fn should_distill(&self, now_ms: i64, activities_changed: bool) -> bool {
        activities_changed
            && (self.distillation_backlog
                || self
                    .last_distillation_at_ms
                    .is_none_or(|last| now_ms.saturating_sub(last) >= CONCLUSION_MIN_INTERVAL_MS))
    }

    /// Record a pass attempted at `now_ms`. A failed pass always waits the full
    /// interval; a successful one drains back to back (one per tick, at the
    /// worker's normal pacing) while changes remain behind the watermark.
    pub(super) fn record_distillation(
        &mut self,
        now_ms: i64,
        succeeded: bool,
        backlog_remaining: bool,
    ) {
        self.last_distillation_at_ms = Some(now_ms);
        self.distillation_backlog = succeeded && backlog_remaining;
    }

    /// Whether this tick should run the confidence-decay beat: every Mth tick.
    /// Decay is the slowest beat and unconditional on new Activities —
    /// silence is precisely when a Conclusion should fade.
    pub(super) fn should_decay(&self) -> bool {
        self.ticks_since_decay >= CONFIDENCE_DECAY_EVERY_M_TICKS
    }
}

/// Run one Conclusion distillation pass over accumulated Activities and stamp a
/// single `derivation_run` (kind `'conclusion'`) with the outcome — including
/// the per-gate withheld counts, so a pass whose drafts were all dropped by
/// policy stays diagnosable. Resilient: any engine/store error records a
/// `failed` run and returns `None`; it never panics the worker. Returns the
/// distillation outcome on success.
///
/// Shared with the run-now command so manual and automatic distillation behave
/// identically and both stamp the same ledger row.
pub(crate) async fn run_conclusion_distillation(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    core: &AppCore,
    subject_vectors: &app_infra::SubjectVectorStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> Option<derivation::ConclusionDistillationOutcome> {
    match derivation::distill_conclusions(engine, store, core, subject_vectors).await {
        Ok(outcome) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "conclusion".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "completed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: outcome.upserted as i64,
                    input_tokens: outcome.input_tokens,
                    output_tokens: outcome.output_tokens,
                    provider: provider_label,
                    model: model_label,
                    error: None,
                    gate_drops: outcome.gate_drops,
                })
                .await;
            if outcome.upserted > 0 {
                crate::debug_log::log(format!(
                    "user context worker distilled {} conclusion(s)",
                    outcome.upserted
                ));
            }
            if outcome.gate_drops.total() > 0 {
                let drops = outcome.gate_drops;
                crate::debug_log::log(format!(
                    "user context distillation withheld {} draft(s): {} ungrounded, \
                     {} guardrail-suppressed, {} below the formation bar, {} resurface-blocked",
                    drops.total(),
                    drops.ungrounded,
                    drops.guardrail_suppressed,
                    drops.below_formation_bar,
                    drops.resurface_blocked,
                ));
            }
            // ADR 0046: supersede activity is an OUTCOME, not a withholding — logged
            // separately from the "withheld N drafts" line so retirements are visible.
            let drops = outcome.gate_drops;
            if drops.superseded + drops.supersede_degraded + drops.supersede_blocked > 0 {
                crate::debug_log::log(format!(
                    "user context supersede: {} retired, {} degraded, {} blocked",
                    drops.superseded, drops.supersede_degraded, drops.supersede_blocked,
                ));
            }
            Some(outcome)
        }
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "conclusion".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error),
                    gate_drops: DistillationGateDrops::default(),
                })
                .await;
            None
        }
    }
}

/// Run one **confidence-decay** pass (#95): the slowest, LLM-free beat. For each
/// decayable Conclusion (`visible`/`faded`, never dismissed), apply the
/// recency-weighted half-life [`confidence::decay`] over elapsed silence,
/// recompute its [`confidence::status_for`] (below the display floor → `faded`,
/// leaving the visible dossier while its Confidence History is kept), persist the
/// new value + status + `last_decayed_at_ms`, and append a Confidence History
/// snapshot so the Subject trajectory can plot the arc. After the loop, prune the
/// history to [`MAX_SNAPSHOTS_PER_CONCLUSION`] per Conclusion. Records a single
/// `derivation_run` (kind `'confidence'`, **0 tokens** — decay uses no LLM) with
/// the count touched. Resilient: every error is swallowed-with-log; it never
/// panics the worker.
///
/// A **pinned** Conclusion is exempt from decay (#99): `list_decayable_conclusions`
/// already drops pinned rows from the set entirely, and the per-row `pinned` flag
/// is passed to `decay()`/`status_for()` (which also honor it) as a belt-and-braces
/// guard. So pinned Conclusions are never touched by this beat.
pub(crate) async fn run_confidence_decay(
    store: &UserContextStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> bool {
    let now = now_ms();
    let conclusions = match store.list_decayable_conclusions().await {
        Ok(conclusions) => conclusions,
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "confidence".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error.to_string()),
                    gate_drops: DistillationGateDrops::default(),
                })
                .await;
            return false;
        }
    };

    let mut touched = 0i64;
    for decayable in &conclusions {
        let conclusion = &decayable.conclusion;
        // A pinned Conclusion is exempt from decay (#99). `list_decayable_conclusions`
        // already excludes pinned rows, so this is normally `false` here; passing the
        // real flag keeps `decay()`/`status_for()` correct as a guard regardless.
        let pinned = conclusion.pinned;
        // Decay over the per-pass DELTA: anchor on `decay_anchor_ms`
        // (`COALESCE(last_decayed_at_ms, last_supported_at_ms)`), not the fixed
        // `last_supported_at_ms`. Each pass feeds back the already-decayed
        // `confidence`; anchoring on the previous decay time makes the per-pass
        // factors telescope to exactly one 30-day half-life instead of re-applying
        // the full elapsed factor every pass (the HIGH compounding bug #1).
        let decayed = confidence::decay(
            conclusion.confidence,
            decayable.decay_anchor_ms,
            now,
            pinned,
        );
        let status = confidence::status_for(decayed, pinned);

        // Stamp `last_decayed_at_ms = now` so the NEXT pass anchors here (this is
        // what `update_conclusion_confidence` writes into `last_decayed_at_ms`).
        if let Err(error) = store
            .update_conclusion_confidence(conclusion.id, decayed, status, now)
            .await
        {
            crate::debug_log::log_warn(format!(
                "user context confidence decay: update failed for conclusion {}: {error}",
                conclusion.id
            ));
            continue;
        }
        // Snapshot the (possibly-faded) value so the Subject trajectory keeps the
        // arc even below the display floor (faded is not deleted).
        if let Err(error) = store
            .insert_confidence_snapshot(conclusion.id, decayed, now)
            .await
        {
            crate::debug_log::log_warn(format!(
                "user context confidence decay: snapshot failed for conclusion {}: {error}",
                conclusion.id
            ));
        }
        touched += 1;
    }

    // Aggressively prune the history tail (recency-weighting means old snapshots
    // stop mattering). Best-effort: a prune error does not fail the beat.
    if let Err(error) = store
        .prune_confidence_history(MAX_SNAPSHOTS_PER_CONCLUSION)
        .await
    {
        crate::debug_log::log_warn(format!(
            "user context confidence decay: history prune failed: {error}"
        ));
    }

    let _ = store
        .insert_derivation_run(NewDerivationRun {
            kind: "confidence".to_string(),
            window_start_ms: None,
            window_end_ms: None,
            status: "completed".to_string(),
            activities_derived: 0,
            conclusions_derived: touched,
            input_tokens: 0,
            output_tokens: 0,
            provider: provider_label,
            model: model_label,
            error: None,
            gate_drops: DistillationGateDrops::default(),
        })
        .await;

    if touched > 0 {
        crate::debug_log::log(format!(
            "user context worker decayed {touched} conclusion(s)"
        ));
    }
    touched > 0
}

/// One worker tick. Returns the sleep to wait before the next tick. All errors
/// are logged, never propagated. `cadence` carries the slower Conclusion beat
/// across ticks.
pub async fn worker_tick(
    infra: &AppInfraState,
    core: &AppCore,
    cadence: &mut WorkerCadence,
) -> Duration {
    let Some(settings_state) = core.try_state::<RecordingSettingsState>() else {
        return IDLE_INTERVAL;
    };
    let settings = current_recording_settings(&settings_state);
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Two-layer gate: continuous derivation runs ONLY when User Context's own
    // opt-in is on (the continuous-derivation opt-in, independent of Ask AI) AND
    // the shared engine-configured prerequisite is satisfied. Cheap-when-off: the
    // opt-in is off by default, so a tick on a fresh install does nothing but
    // idle (this short-circuits before the prerequisite's local reachability
    // ping).
    if !user_context.enabled {
        return IDLE_INTERVAL;
    }

    if crate::ai_runtime::engine_configured_prerequisite(&ai_runtime)
        .await
        .is_err()
    {
        // Engine off / not ready (no model / no key / unreachable endpoint).
        // Stay cheap; do not log the reason at error level on every tick.
        return IDLE_INTERVAL;
    }

    let engine = match crate::ai_runtime::resolve_engine_config_live(&ai_runtime, None, None).await
    {
        Ok(engine) => engine,
        Err(_reason) => {
            // Defensive: the prerequisite already passed, but a local engine
            // could become unreachable between the ping and here. Stay cheap.
            return IDLE_INTERVAL;
        }
    };

    // Pacing + run-ledger labels key off the global default model's provider
    // (the prerequisite guarantees one is chosen; defensive bail otherwise).
    // Pacing depends on the provider *kind* (cloud vs local), so resolve the
    // default model's provider instance id back to its kind via the provider
    // list.
    let Some(default_provider) = default_provider_kind(&ai_runtime) else {
        return IDLE_INTERVAL;
    };
    let provider_label = provider_label_for(&ai_runtime);
    let model_label = model_label_for(&ai_runtime);

    // --- Activity beat (frequent) ---
    let run = run_forward_activity_window(
        &engine,
        infra.user_context(),
        provider_label.clone(),
        model_label.clone(),
    )
    .await;

    // Catch-up signal (C): if this tick covered a forward window and the cursor
    // is still more than one window behind realtime, we have a backlog of
    // windows-with-captures to drain — sleep the short catch-up interval (below)
    // and defer the expensive Conclusion distillation until caught up.
    let forward_backlog_ms = now_ms().saturating_sub(run.window_end_ms);
    let catching_up = is_catching_up(run.derived_window, forward_backlog_ms);

    let mut dossier_changed = run.changed;
    if run.activities_derived > 0 {
        crate::debug_log::log(format!(
            "user context worker derived {} activities (window=[{},{}], items={})",
            run.activities_derived, run.window_start_ms, run.window_end_ms, run.items_read
        ));
    }

    // --- History Backfill beat (backward, newest-first) ---
    // (#98) STRICTLY newest-first: only extend coverage *backward* once the forward
    // catch-up has reached realtime this tick (`!catching_up`). Recency-weighted
    // Confidence means recent history drives current conclusions, so recent
    // windows are always covered before older ones. NOT `!run.derived_window`: a
    // tick interval (≥ 120 s) never undercuts `MIN_WINDOW_MS`, so forward always
    // has a fresh window and that gate starved backfill for good.
    //
    // This is a BACKGROUND TRICKLE paced by the Derivation Budget — one bounded
    // window per pass, up to a small per-tier count — never a synchronous
    // whole-history bill (there is deliberately no "derive all of history now"
    // path). It is bounded by default (`backfill_window_days`) and extends to the
    // true earliest capture only under the explicit go-deeper toggle.
    if !catching_up {
        let now = now_ms();
        let earliest_capture = if user_context.backfill_go_deeper {
            infra
                .user_context()
                .earliest_capture_at_ms()
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let floor_ms = backfill_floor_ms(
            now,
            user_context.backfill_window_days,
            user_context.backfill_go_deeper,
            earliest_capture,
        );
        let max_windows =
            backfill_windows_per_tick(default_provider, user_context.derivation_budget_tier);
        let backfilled = run_backfill_pass(
            &engine,
            infra.user_context(),
            floor_ms,
            max_windows,
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        if backfilled {
            dossier_changed = true;
            crate::debug_log::log(
                "user context worker backfilled older history (background trickle)",
            );
        }
    }

    // --- Failed-window Retry beat (#113, lowest-priority trickle) ---
    // Re-derive windows whose only coverage is `failed` runs (a transient cloud
    // hiccup mid-window would otherwise leave a PERMANENT hole in the Activity
    // history — the cursor advances past failures by design). This runs every
    // tick AFTER the forward beat (fresh capture always derives first; gating it
    // on an idle forward beat would starve retries forever during continuous
    // use), is capped at one window per tick, and the store query enforces the
    // per-window attempt cap + wall-clock backoff so a deterministically-failing
    // window stops consuming engine calls.
    {
        let retried = run_failed_window_retry_pass(
            &engine,
            infra.user_context(),
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        if retried {
            dossier_changed = true;
        }
    }

    // --- Conclusion beat (slower) ---
    // CADENCE HOOK (spec §6 / §4): after the Activity passes, run Conclusion
    // distillation at most once per `CONCLUSION_MIN_INTERVAL_MS`, and only when
    // an Activity was inserted or extended since the last successful pass (the
    // durable watermark, so a restart neither loses nor repeats changes). It
    // shares the SAME resolved engine and records its own `derivation_run`
    // (kind `'conclusion'`).
    // While draining a forward backlog (catch-up cadence) we derive many
    // Activities fast; deferring distillation until caught up avoids an LLM
    // distillation call on every catch-up tick — the changes wait behind the
    // watermark for the first normal tick afterward.
    let now = now_ms();
    let activities_changed = !catching_up
        && infra
            .user_context()
            .activities_changed_since_distillation()
            .await
            .unwrap_or(false);
    if cadence.should_distill(now, activities_changed) {
        let outcome = run_conclusion_distillation(
            &engine,
            infra.user_context(),
            core,
            infra.subject_vectors(),
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        dossier_changed = dossier_changed || outcome.is_some_and(|outcome| outcome.upserted > 0);
        // Still changed right after a pass that actually sent something = the
        // pass hit its cap. (A no-op pass — e.g. a lone Activity — sends nothing
        // and must not spin every tick.)
        let backlog_remaining = outcome.is_some_and(|outcome| outcome.input_tokens > 0)
            && infra
                .user_context()
                .activities_changed_since_distillation()
                .await
                .unwrap_or(false);
        cadence.record_distillation(now, outcome.is_some(), backlog_remaining);
    }

    // --- Confidence-decay beat (slowest, LLM-free) ---
    // (#95) Every Mth tick, fade Conclusions on their recency-weighted half-life,
    // recompute faded/visible status, snapshot Confidence History, and prune it.
    // This beat counts ticks (not Activities) and runs even when no new Activities
    // arrived — silence is exactly when a Conclusion should fade. It uses NO LLM,
    // so it records a 0-token `derivation_run` (kind `'confidence'`).
    cadence.ticks_since_decay = cadence.ticks_since_decay.saturating_add(1);
    if cadence.should_decay() {
        let changed = run_confidence_decay(infra.user_context(), provider_label, model_label).await;
        dossier_changed = dossier_changed || changed;
        cadence.ticks_since_decay = 0;
    }

    if dossier_changed {
        core.host().emit(CoreEvent::UserContextChanged);
    }

    // Catch-up cadence (C): drain a remaining forward backlog of
    // windows-with-captures back-to-back; otherwise sleep the tier-paced interval.
    if catching_up {
        CATCHUP_INTERVAL
    } else {
        tick_interval(default_provider, user_context.derivation_budget_tier)
    }
}

/// Provider label for the run ledger: the global default model's stable
/// provider id (background derivation always uses the global default model).
pub(crate) fn provider_label_for(settings: &capture_types::AiRuntimeSettings) -> Option<String> {
    settings
        .default_model
        .as_ref()
        .map(|model| model.provider.clone())
}

/// The global default model's provider *kind*, resolved from its instance id
/// through the provider list (`None` when no default or a dangling id).
pub fn default_provider_kind(
    settings: &capture_types::AiRuntimeSettings,
) -> Option<capture_types::AiProviderKind> {
    let model = settings.default_model.as_ref()?;
    settings
        .providers
        .iter()
        .find(|provider| provider.id == model.provider)
        .map(|provider| provider.kind)
}

/// Model label for the run ledger (the global default model id).
pub(crate) fn model_label_for(settings: &capture_types::AiRuntimeSettings) -> Option<String> {
    settings
        .default_model
        .as_ref()
        .map(|model| model.model.trim().to_string())
        .filter(|model| !model.is_empty())
}
