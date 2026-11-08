use super::embedder::*;
use super::sweep::*;
use super::*;
use semantic_search::{builtin_model_manifest, SemanticSearchPooling};

#[test]
fn descriptor_pooling_is_hand_coded_per_model() {
    // Pooling rides the hand-coded descriptor (ADR 0037), never a guess from the
    // model id: nomic/e5 are Mean, bge-m3 is Cls. The worker loads each model
    // through `load_from_dir(&install_dir, descriptor)`, so the candle backend
    // pools with exactly `descriptor.pooling` — this pins the declared value the
    // loader is handed.
    let pooling_for = |slug: &str| -> SemanticSearchPooling {
        resolve_descriptor(semantic_search::SEMANTIC_SEARCH_PROVIDER_ID, slug)
            .unwrap_or_else(|| panic!("{slug} must resolve"))
            .pooling
    };

    assert_eq!(pooling_for("nomic-embed-text-v1.5"), SemanticSearchPooling::Mean);
    assert_eq!(pooling_for("multilingual-e5-small"), SemanticSearchPooling::Mean);
    assert_eq!(pooling_for("bge-m3"), SemanticSearchPooling::Cls);
}

#[test]
fn resolve_descriptor_honors_enabled_and_known_model() {
    let mut settings = default_semantic_search_settings();
    // Default English tier resolves to the nomic 768-dim descriptor.
    let descriptor = resolve_selected_descriptor(&settings).expect("default descriptor");
    assert_eq!(descriptor.model_id, "nomic-embed-text-v1.5");
    assert_eq!(descriptor.dimension, 768);

    // Disabled => no descriptor (the worker idles).
    settings.enabled = false;
    assert!(resolve_selected_descriptor(&settings).is_none());

    // Unknown model => no descriptor.
    settings.enabled = true;
    settings.model_id = Some("not-a-real-model".to_string());
    assert!(resolve_selected_descriptor(&settings).is_none());

    // No model selected => no descriptor.
    settings.model_id = None;
    assert!(resolve_selected_descriptor(&settings).is_none());
}

#[test]
fn embedder_matches_only_the_same_provider_and_model() {
    let manifest = builtin_model_manifest();
    let nomic = manifest
        .models
        .iter()
        .find(|m| m.model_id == "nomic-embed-text-v1.5")
        .expect("nomic descriptor");

    // No loaded embedder => never matches.
    assert!(!embedder_matches(&None, nomic));
}

/// The sweep's store gate (`run_sweep_pass`) stamps every vector under
/// `descriptor.model_id` — the model it INTENDED to load — so if this reuse
/// predicate wrongly hands back a different model's shared instance, model A's
/// vectors are written under model B's stamp and the vec0 epoch stamp never
/// catches it. This predicate, not the stamp, is the last line of defence.
#[test]
fn the_shared_embedder_is_reused_only_for_the_exact_selection() {
    let nomic =
        resolve_descriptor(semantic_search::SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5")
            .expect("nomic must resolve");

    // Fresh + exact provider/model match: reuse (the point of the shared slot).
    assert!(shared_embedder_is_reusable(
        &nomic.provider,
        &nomic.model_id,
        Duration::ZERO,
        &nomic
    ));

    // A different model at the SAME 768 width (gte-modernbert-base vs nomic):
    // still a miss — only the id distinguishes them, a width check cannot.
    assert!(!shared_embedder_is_reusable(
        &nomic.provider,
        "gte-modernbert-base",
        Duration::ZERO,
        &nomic
    ));

    // A different provider: miss.
    assert!(!shared_embedder_is_reusable(
        "some-other-provider",
        &nomic.model_id,
        Duration::ZERO,
        &nomic
    ));

    // Warm past the shared cap: miss, so recycling actually frees the weights.
    assert!(!shared_embedder_is_reusable(
        &nomic.provider,
        &nomic.model_id,
        MAX_EMBEDDER_WARM_LIFETIME,
        &nomic
    ));
}

#[test]
fn idle_drop_releases_the_embedder_exactly_once_after_the_grace_period() {
    // The "decay to idle" lever: a caught-up worker holds the model weights
    // resident, so after a grace period of consecutive idle passes the embedder
    // must be released — but only once per idle stretch, not every tick.
    let mut idles = 0u32;

    // Idle passes short of the grace threshold keep the embedder warm.
    for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
        assert!(
            !advance_idle_drop(&mut idles, false, true),
            "must not release before the grace threshold"
        );
    }
    // The Nth consecutive idle pass releases it (caller drops the embedder).
    assert_eq!(idles, IDLE_PASSES_BEFORE_EMBEDDER_DROP - 1);
    assert!(
        advance_idle_drop(&mut idles, false, true),
        "releases exactly at the grace threshold"
    );

    // After the drop the embedder is gone: further idle passes must NOT signal a
    // release again (no re-drop, no re-log) — once per idle stretch.
    assert!(
        !advance_idle_drop(&mut idles, false, false),
        "no second release while already unloaded"
    );
    assert!(!advance_idle_drop(&mut idles, false, false));
}

#[test]
fn idle_drop_streak_resets_on_any_work_pass() {
    // A single pass that embeds something resets the streak, so a worker that is
    // still trickling through a backlog never sheds the warm embedder mid-drain.
    let mut idles = 0u32;

    // Idle right up to the brink...
    for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
        assert!(!advance_idle_drop(&mut idles, false, true));
    }
    // ...then a work pass resets the streak (and never releases on work).
    assert!(!advance_idle_drop(&mut idles, true, true));
    assert_eq!(idles, 0, "work resets the idle streak");

    // The next idle stretch must start counting from zero, not fire immediately.
    for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
        assert!(
            !advance_idle_drop(&mut idles, false, true),
            "post-work idle streak restarts from zero"
        );
    }
    assert!(advance_idle_drop(&mut idles, false, true));
}

#[test]
fn idle_drop_never_fires_without_a_loaded_embedder() {
    // The no-model / already-unloaded idle path: a worker with nothing loaded
    // (e.g. no Semantic Search Model installed) idles forever without ever
    // signalling a release, no matter how long the streak grows.
    let mut idles = 0u32;
    for _ in 0..(IDLE_PASSES_BEFORE_EMBEDDER_DROP + 5) {
        assert!(
            !advance_idle_drop(&mut idles, false, false),
            "nothing to release when no embedder is loaded"
        );
    }
}

#[test]
fn anchor_is_quarantined_only_after_n_consecutive_failures() {
    // L3: a deterministically-failing anchor must be retried up to the cap and
    // quarantined exactly at it — never retried forever.
    let mut state = SweepState::new();
    let anchor_id = 42;

    assert!(!state.is_anchor_quarantined(anchor_id), "clean anchor is eligible");

    // Each failure short of the cap leaves the anchor still eligible to retry.
    for expected in 1..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
        assert_eq!(failures, expected);
        assert!(!now_quarantined, "not quarantined before the cap");
        assert!(!state.is_anchor_quarantined(anchor_id));
    }

    // The Nth consecutive failure quarantines it.
    let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
    assert_eq!(failures, MAX_CONSECUTIVE_ANCHOR_FAILURES);
    assert!(now_quarantined, "quarantined exactly at the cap");
    assert!(state.is_anchor_quarantined(anchor_id));
}

#[test]
fn a_clean_store_resets_an_anchor_failure_streak() {
    // A transient blip that later succeeds must not accumulate toward
    // quarantine: clearing the streak resets the counter.
    let mut state = SweepState::new();
    let anchor_id = 7;

    state.record_anchor_embed_failure(anchor_id);
    state.record_anchor_embed_failure(anchor_id);
    // A successful store (or delete/reprocess) clears the streak.
    state.clear_anchor_failures(anchor_id);
    assert!(!state.is_anchor_quarantined(anchor_id));

    // It now takes the full cap of consecutive failures again to quarantine.
    for _ in 1..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        assert!(!state.record_anchor_embed_failure(anchor_id).1);
    }
    assert!(state.record_anchor_embed_failure(anchor_id).1);
}

#[test]
fn reprocessing_an_anchor_with_a_new_id_escapes_quarantine() {
    // A reprocess deletes + reinserts the search projection with a NEW
    // search_documents.id; the in-memory quarantine is keyed by id, so the
    // replacement id is simply absent from the map and is retried. This is the
    // "retry only on reprocess" convention the persistent workers express via a
    // new row.
    let mut state = SweepState::new();
    let old_id = 100;
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(old_id);
    }
    assert!(state.is_anchor_quarantined(old_id), "old id is quarantined");

    let new_id = 101; // the reprocessed anchor's fresh id
    assert!(
        !state.is_anchor_quarantined(new_id),
        "the reprocessed id is not quarantined and is retried"
    );
}

#[test]
fn health_snapshot_records_a_load_failure_and_clears_it_on_a_successful_pass() {
    // The debug surface's whole view of this worker is this snapshot, so the two
    // transitions it must survive are: a load failure (streak + the error string
    // that used to be logged and dropped) and the recovery that clears them.
    let mut state = SweepState::new();

    // Nothing has happened yet: no model held, nothing failed, nothing quarantined.
    let fresh = state.health_snapshot();
    assert!(!fresh.model_loaded);
    assert_eq!(fresh.consecutive_load_failures, 0);
    assert_eq!(fresh.quarantined_count, 0);
    assert_eq!(fresh.last_load_error, None);

    // Two consecutive load failures: the streak climbs and the LATEST error is the
    // one surfaced.
    assert_eq!(state.record_load_failure("weights are truncated"), 1);
    assert_eq!(state.record_load_failure("device init failed"), 2);
    let failed = state.health_snapshot();
    assert_eq!(failed.consecutive_load_failures, 2);
    assert_eq!(failed.last_load_error.as_deref(), Some("device init failed"));
    assert!(!failed.model_loaded, "a failed load holds no embedder");

    // A successful pass (the `Ok(pair)` arm) clears the streak AND the stale error,
    // so the debug page stops showing a load error the worker has recovered from.
    state.clear_load_failures();
    let recovered = state.health_snapshot();
    assert_eq!(recovered.consecutive_load_failures, 0);
    assert_eq!(recovered.last_load_error, None);
    assert_eq!(state.corrupt_model_signalled, None, "the CT3 latch clears too");
}

#[test]
fn health_snapshot_counts_only_quarantined_anchors_not_sub_cap_streaks() {
    // `anchor_failures` holds every failing anchor's streak, but only those AT the
    // cap are actually excluded from the sweep — the snapshot must report the
    // quarantine, not the raw map size, or the debug page overstates the damage.
    let mut state = SweepState::new();

    // One anchor short of the cap: failing, but still being retried.
    state.record_anchor_embed_failure(1);
    assert_eq!(
        state.health_snapshot().quarantined_count,
        0,
        "a sub-cap failure streak is not a quarantine"
    );

    // A second anchor all the way to the cap: quarantined.
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(2);
    }
    assert_eq!(state.health_snapshot().quarantined_count, 1);

    // A clean store on the quarantined id releases it from the count.
    state.clear_anchor_failures(2);
    assert_eq!(state.health_snapshot().quarantined_count, 0);
}

#[test]
fn corrupt_latch_is_keyed_to_the_signalled_model_identity() {
    // CT3: the corrupt-model latch must only short-circuit the selection it was
    // raised for. A latch for model A does not idle a switch to model B.
    let mut state = SweepState::new();
    let provider = "mnema";
    let model_a = "corrupt-model";
    let model_b = "good-model";

    // No latch initially: nothing matches.
    assert!(!state.corrupt_latch_matches(provider, model_a));

    // Latch model A corrupt (as the load-failure cap site does).
    state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));
    state.consecutive_load_failures = MAX_CONSECUTIVE_LOAD_FAILURES;
    // Pretend a stale embedder for A is cached.
    assert!(state.corrupt_latch_matches(provider, model_a));
    assert!(
        !state.corrupt_latch_matches(provider, model_b),
        "a different model is never matched by A's latch"
    );

    // Reconciling against the SAME model is a no-op: the latch (and counter) hold.
    state.reconcile_selection(provider, model_a);
    assert_eq!(
        state.corrupt_model_signalled,
        Some((provider.to_string(), model_a.to_string()))
    );
    assert_eq!(state.consecutive_load_failures, MAX_CONSECUTIVE_LOAD_FAILURES);
    assert!(state.corrupt_latch_matches(provider, model_a));
}

#[test]
fn switching_models_clears_a_corrupt_latch_raised_for_the_old_model() {
    // The regression FIX 1 fixes: after model A is flagged corrupt, switching to
    // a valid model B must clear the latch (and the load counter, and the stale
    // cached embedder) so B is loaded instead of the worker idling forever.
    let mut state = SweepState::new();
    let provider = "mnema";
    let model_a = "corrupt-model";
    let model_b = "good-model";

    state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));
    state.consecutive_load_failures = MAX_CONSECUTIVE_LOAD_FAILURES;

    // The user switches to model B: reconcile clears the latch keyed to A.
    state.reconcile_selection(provider, model_b);
    assert_eq!(state.corrupt_model_signalled, None, "A's latch is cleared on switch");
    assert_eq!(state.consecutive_load_failures, 0, "load counter resets for the new model");
    assert!(
        !state.corrupt_latch_matches(provider, model_b),
        "model B is not short-circuited and proceeds to load"
    );
}

#[test]
fn a_whole_batch_embed_failure_does_not_quarantine_each_anchor() {
    // FIX 2: a transient whole-batch fault surfaces as MORE THAN ONE failing
    // anchor in the batch, and must be treated as transient (no per-anchor
    // quarantine), while a genuine single poison anchor (one failure, healthy
    // siblings) is still credited toward quarantine.
    assert!(
        !is_isolated_embed_failure(0),
        "no failures => nothing to credit"
    );
    assert!(
        is_isolated_embed_failure(1),
        "exactly one anchor failing in isolation is a genuine per-anchor fault"
    );
    for batch_failures in 2..=SWEEP_BATCH_SIZE as usize {
        assert!(
            !is_isolated_embed_failure(batch_failures),
            "{batch_failures} anchors failing together is a transient batch fault, not poison"
        );
    }
}

#[test]
fn backfill_cooldown_clamps_work_time_to_the_pacing_band() {
    // CPU pacing: the inter-batch cooldown is the batch wall time scaled and
    // clamped to [MIN, MAX], so a fast batch still yields the cores (floor) and
    // a slow batch does not stall the backlog (ceiling).
    assert_eq!(
        backfill_batch_cooldown(Duration::ZERO),
        BACKFILL_BATCH_COOLDOWN_MIN,
        "a trivially fast batch still pays the floor (the old 0ms gave none)"
    );
    assert_eq!(
        backfill_batch_cooldown(Duration::from_secs(60)),
        BACKFILL_BATCH_COOLDOWN_MAX,
        "a very slow batch is capped at the ceiling"
    );
    // A mid-band batch stays inside the clamp band. (The multiplier itself is
    // pinned by literal in
    // `the_cooldown_band_pins_the_shipped_duty_target_with_a_literal` —
    // `cooldown(mid) == mid * MULTIPLIER` here would restate the
    // implementation against itself and pass for ANY multiplier.)
    let mid = Duration::from_millis(500);
    assert!(backfill_batch_cooldown(mid) >= BACKFILL_BATCH_COOLDOWN_MIN);
    assert!(backfill_batch_cooldown(mid) <= BACKFILL_BATCH_COOLDOWN_MAX);

    // REGRESSION GUARD: a REALISTIC batch must not reach the ceiling. A batch of
    // SWEEP_BATCH_SIZE real anchors measures ~3.5s on an M4 Air, and if the cap
    // binds there it becomes the governor instead of the multiplier — which is
    // the exact bug this band was retuned to fix (duty pinned at 65%, and RISING
    // to 86% when the GPU throttled, because a slower batch still got the same
    // fixed rest). Keep MAX well above `multiplier * typical_batch`.
    let realistic = Duration::from_secs(4);
    assert!(
        backfill_batch_cooldown(realistic) < BACKFILL_BATCH_COOLDOWN_MAX,
        "MAX must stay a safety bound, not the governor, for a realistic batch"
    );
    // ...and the duty cycle it produces is the multiplier's target, so a slower
    // forward earns proportionally more rest (self-correcting under throttle).
    let cooldown = backfill_batch_cooldown(realistic);
    let duty = realistic.as_secs_f64() / (realistic + cooldown).as_secs_f64();
    let target = 1.0 / (1.0 + BACKFILL_BATCH_COOLDOWN_MULTIPLIER);
    assert!(
        (duty - target).abs() < 0.01,
        "duty {duty:.3} should track the multiplier's target {target:.3}"
    );
}

/// The cooldown input is the FORWARD alone, routed through [`paced_input`].
/// What it must NOT pace over is the same task's model (re)load — one pass per
/// `MAX_EMBEDDER_WARM_LIFETIME` reloads the weights (547 MB of F32 safetensors
/// cast into device memory for the default model) in the same blocking task,
/// and a reload alone eats the entire pacing band (`MAX / multiplier` = 10s) —
/// nor the caller's DB tail (per-anchor re-checks + the batch write). Timing
/// the whole region would clamp a recycle pass at the ceiling AND rest 3x a
/// disk read that is not the GPU burn the duty cycle governs.
#[test]
fn the_pacing_input_is_the_forward_alone() {
    // Realistic magnitudes: a warm-cap reload, a SWEEP_BATCH_SIZE forward
    // (measured ~1.7s on an M4 Air), the DB tail.
    let load = Duration::from_secs(10);
    let forward = Duration::from_millis(1_700);
    let db_tail = Duration::from_secs(2);

    assert_eq!(
        paced_input(load, forward, db_tail),
        forward,
        "the pacing input is the forward alone"
    );
    assert!(
        backfill_batch_cooldown(paced_input(load, forward, db_tail))
            < BACKFILL_BATCH_COOLDOWN_MAX,
        "the forward scales through unclamped — the multiplier stays the governor"
    );
    assert_eq!(
        backfill_batch_cooldown(load + forward + db_tail),
        BACKFILL_BATCH_COOLDOWN_MAX,
        "timing the whole region would clamp a recycle pass at the ceiling"
    );
}

#[test]
fn clustered_poison_trips_per_anchor_isolation_after_repeated_transient_passes() {
    // F3: a >=2-anchor poison cluster sharing the newest-first window fails the
    // batched embed as a whole-batch (transient) fault every pass, so it never
    // shrinks. After a bounded number of transient-only passes over the SAME set,
    // the detector must flip the next pass to per-anchor isolation so each poison
    // anchor fails alone and can be credited toward quarantine.
    let mut state = SweepState::new();
    let window = vec![10i64, 11, 12]; // a clustered window (order should not matter)

    // The first transient passes stay on the batched path (not yet stuck).
    for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
        assert!(
            !state.is_stuck_on(&window),
            "must not isolate before the transient-pass threshold"
        );
        state.record_transient_pass(window.clone());
    }
    // Now stuck on this exact window => isolate next pass (order-insensitive).
    assert!(state.is_stuck_on(&window), "stuck after the threshold");
    let reordered = vec![12i64, 10, 11];
    assert!(
        state.is_stuck_on(&reordered),
        "the signature is the SET, not the order"
    );
}

#[test]
fn a_different_transient_window_restarts_the_isolation_streak() {
    // A single transient blip (or a window that slid because progress was made
    // elsewhere) must NOT accrue toward isolation: a different set restarts the
    // streak, so only a genuinely STUCK window ever reaches per-anchor isolation.
    let mut state = SweepState::new();

    for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
        state.record_transient_pass(vec![1, 2, 3]);
    }
    assert!(state.is_stuck_on(&[1, 2, 3]), "stuck on the original window");

    // The window changes (the backlog slid): the streak restarts from this set.
    state.record_transient_pass(vec![4, 5, 6]);
    assert!(
        !state.is_stuck_on(&[4, 5, 6]),
        "a fresh window is not immediately stuck"
    );
    assert!(
        !state.is_stuck_on(&[1, 2, 3]),
        "the old window is no longer the tracked signature"
    );
}

#[test]
fn clearing_the_detector_after_progress_resets_isolation() {
    // Any progress pass (store / quarantine) clears the detector, so a later
    // transient stall starts a fresh streak rather than inheriting a stale one.
    let mut state = SweepState::new();
    for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
        state.record_transient_pass(vec![7, 8]);
    }
    assert!(state.is_stuck_on(&[7, 8]));

    state.clear_transient_stuck();
    assert!(!state.is_stuck_on(&[7, 8]), "progress resets the isolation streak");
}

#[test]
fn switching_models_clears_the_per_anchor_quarantine_map() {
    // F4: a quarantine recorded under model A is model-specific (a
    // tokenizer-incompatible input under A may embed fine under B). On a model
    // switch the vec0 table is rebuilt and every anchor re-enters the missing set,
    // so a stale quarantine from A would wrongly filter that anchor from every
    // batch under B until restart. Reconciling to a different selection must clear
    // the map (and the clustered-poison detector, and the idle streak).
    let mut state = SweepState::new();
    let provider = "mnema";
    let model_a = "model-a";
    let model_b = "model-b";

    // Quarantine an anchor under model A and arrive at a stuck/idle posture.
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(999);
    }
    assert!(state.is_anchor_quarantined(999), "quarantined under model A");
    state.record_transient_pass(vec![1, 2]);
    state.consecutive_idles = IDLE_PASSES_BEFORE_EMBEDDER_DROP;
    // The latch must be set for the different-selection branch to run.
    state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));

    // Switch to model B.
    state.reconcile_selection(provider, model_b);
    assert!(
        !state.is_anchor_quarantined(999),
        "the quarantine map is cleared on a model switch so B retries every anchor"
    );
    assert!(
        !state.is_stuck_on(&[1, 2]),
        "the clustered-poison detector is cleared on a model switch"
    );
    assert_eq!(
        state.consecutive_idles, 0,
        "the idle streak resets on a model switch"
    );
}

#[test]
fn a_clean_model_switch_without_a_corrupt_latch_still_clears_the_quarantine_map() {
    // F4 (the common path): the per-anchor quarantine map must be cleared on ANY
    // model switch, not only one where a corrupt-model latch happened to be set.
    // `reconcile_selection` keys off the tracked previous selection, so a clean
    // switch (no latch ever raised) is detected and the model-specific state is
    // cleared for the new model.
    let mut state = SweepState::new();
    let provider = "mnema";

    // Prime the previous selection by reconciling once to model A (first pass: no
    // switch detected, no state touched).
    state.reconcile_selection(provider, "model-a");
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(321);
    }
    assert!(state.is_anchor_quarantined(321), "quarantined under model A");
    assert!(
        state.corrupt_model_signalled.is_none(),
        "no corrupt latch in this scenario"
    );

    // A clean switch to model B (still no latch) must clear A's quarantine.
    state.reconcile_selection(provider, "model-b");
    assert!(
        !state.is_anchor_quarantined(321),
        "a clean model switch clears the quarantine map keyed to the old model"
    );

    // Reconciling to the SAME model B again is a no-op (no spurious clear).
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(654);
    }
    state.reconcile_selection(provider, "model-b");
    assert!(
        state.is_anchor_quarantined(654),
        "an unchanged selection preserves the quarantine map"
    );
}

#[test]
fn reconcile_to_the_same_model_keeps_the_quarantine_map() {
    // The same-selection branch is a no-op: reconciling to the SAME model must NOT
    // wipe the quarantine map (the model has not changed, so the failures still
    // apply). Only a genuine switch clears it.
    let mut state = SweepState::new();
    let provider = "mnema";
    let model = "model-a";
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(5);
    }
    state.corrupt_model_signalled = Some((provider.to_string(), model.to_string()));

    state.reconcile_selection(provider, model);
    assert!(
        state.is_anchor_quarantined(5),
        "reconciling to the same model preserves its quarantine map"
    );
}

#[test]
fn prune_anchor_failures_drops_entries_absent_from_the_live_missing_set() {
    // F18: the per-pass prune keeps the quarantine map bounded by the live missing
    // set even when continuous capture never lets the backlog reach the empty-
    // backlog bulk clear. An entry whose id is no longer peeked (it stored, or its
    // id was retired by a reprocess) is dropped; an id still peeked is retained.
    let mut state = SweepState::new();
    state.record_anchor_embed_failure(1); // still missing this pass
    state.record_anchor_embed_failure(2); // gone (stored / reprocessed)
    for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
        state.record_anchor_embed_failure(3); // quarantined AND still missing
    }

    let live: std::collections::HashSet<i64> = [1i64, 3].into_iter().collect();
    state.prune_anchor_failures(&live);

    assert!(state.anchor_failures.contains_key(&1), "a still-missing id is kept");
    assert!(
        !state.anchor_failures.contains_key(&2),
        "an id no longer in the missing set is pruned (no uptime-scaled leak)"
    );
    assert!(
        state.is_anchor_quarantined(3),
        "a still-missing quarantined id is retained so it stays filtered out"
    );
}

#[test]
fn backlog_count_runs_only_on_the_coarse_cadence() {
    // F6: the logging-only backlog count must not run every work pass (it is a full
    // COUNT(*) on the shared capture pool). It fires once every N work passes.
    let mut state = SweepState::new();
    let mut counted = 0u32;
    // Run several full cadences worth of work passes.
    let passes = BACKLOG_COUNT_EVERY_N_WORK_PASSES * 3;
    for _ in 0..passes {
        if state.should_count_backlog_this_pass() {
            counted += 1;
        }
    }
    assert_eq!(
        counted, 3,
        "exactly one count per BACKLOG_COUNT_EVERY_N_WORK_PASSES work passes"
    );
}

#[test]
fn error_passes_decay_the_embedder_via_the_idle_drop() {
    // F3 (part b): a persistent error loop must release the model weights after the
    // grace period instead of pinning them resident — the Error arm advances the
    // same idle-drop decay as Idle. `advance_idle_drop` is the shared mechanism;
    // an Error pass calls it with `did_work = false`, so the streak grows and the
    // embedder is released exactly at the grace threshold.
    let mut idles = 0u32;
    for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
        assert!(
            !advance_idle_drop(&mut idles, false, true),
            "an error pass under the grace threshold keeps the embedder warm"
        );
    }
    assert!(
        advance_idle_drop(&mut idles, false, true),
        "a sustained error loop releases the embedder at the grace threshold"
    );
}

#[test]
fn warm_embedder_recycles_at_the_cap_and_not_before() {
    // The cap is what bounds candle's Metal buffer pool during a long drain (the
    // idle-drop never fires there, because every pass is `DidWork`). Two ways it
    // can silently stop doing its job, both pinned here:
    //
    // 1. A unit slip (`from_millis` for `from_secs`) would make it 0.3s, so every
    //    batch would reload the multi-hundred-MB model — a self-inflicted stall
    //    that still "passes" as a memory fix.
    // 2. A cap under one batch's cooldown would recycle between every batch.
    assert!(
        MAX_EMBEDDER_WARM_LIFETIME >= Duration::from_secs(60),
        "the warm cap must be a multi-minute wall clock, not sub-second"
    );
    assert!(
        MAX_EMBEDDER_WARM_LIFETIME > BACKFILL_BATCH_COOLDOWN_MAX,
        "the warm cap must outlast one paced batch or the worker thrashes reloads"
    );

    // Durations, not wound-back `Instant`s: `Instant::checked_sub` returns `None`
    // on a host with less uptime than the cap, which panicked on fresh CI VMs.
    assert!(
        !LoadedEmbedder::stale_after(Duration::ZERO),
        "a freshly loaded embedder stays warm for the drain"
    );
    assert!(
        !LoadedEmbedder::stale_after(MAX_EMBEDDER_WARM_LIFETIME - Duration::from_secs(1)),
        "one second short of the cap is still warm — the boundary is inclusive"
    );
    assert!(
        LoadedEmbedder::stale_after(MAX_EMBEDDER_WARM_LIFETIME),
        "an embedder warm for the whole cap recycles on the next batch"
    );
}

/// The pacing band's assertions must fail on a multiplier drift in EITHER
/// direction. `cooldown(mid) == mid * MULTIPLIER` restates the implementation
/// against itself and passes for any multiplier; `realistic < MAX` only catches a
/// multiplier raised past ~7.5. Nothing bounded it from BELOW, so dropping it to
/// 0.1 (duty 91% — the throttle-amplifying regime this band was retuned to leave)
/// kept every assertion green. One literal pins the shipped duty target.
#[test]
fn the_cooldown_band_pins_the_shipped_duty_target_with_a_literal() {
    assert_eq!(
        backfill_batch_cooldown(Duration::from_secs(4)),
        Duration::from_secs(12),
        "a 4s forward must earn 12s of rest — a 25% duty cycle. This literal is \
         the only assertion that fails if BACKFILL_BATCH_COOLDOWN_MULTIPLIER \
         drifts in either direction."
    );
}

#[test]
fn effective_settings_read_the_recording_settings_the_core_manages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let core = crate::test_support::core_in(Default::default(), dir.path());
    // Early startup, nothing managed yet: the default-on English-tier selection.
    assert_eq!(
        effective_semantic_search_settings(&core),
        default_semantic_search_settings()
    );

    let state = crate::recording_settings::RecordingSettingsState::default();
    core.manage(state.clone());
    // The host keeps its own clone; a change through it is what the worker reads.
    state.lock().unwrap().settings.semantic_search.model_id =
        Some("multilingual-e5-small".to_string());
    assert_eq!(
        effective_semantic_search_settings(&core).model_id.as_deref(),
        Some("multilingual-e5-small")
    );
}

#[test]
fn corrupt_model_signal_reaches_the_host_as_a_failed_download_progress() {
    let host = Arc::new(crate::test_support::FakeHost::default());
    let dir = tempfile::tempdir().expect("tempdir");
    let core = crate::test_support::core_in(host.clone(), dir.path());
    let descriptor = resolve_descriptor(
        semantic_search::SEMANTIC_SEARCH_PROVIDER_ID,
        "nomic-embed-text-v1.5",
    )
    .expect("catalog model");

    signal_model_appears_corrupt(&core, &descriptor, "bad weights");

    let events = host.events.lock().unwrap();
    let [CoreEvent::SemanticSearchModelDownloadProgress(progress)] = events.as_slice() else {
        panic!("expected one progress event, got {events:?}");
    };
    assert_eq!(progress.status, SemanticSearchModelDownloadStatusDto::Failed);
    assert_eq!(progress.model_id, "nomic-embed-text-v1.5");
    assert!(progress
        .message
        .as_deref()
        .is_some_and(|message| message.contains("Reinstall it from Settings")));
}
