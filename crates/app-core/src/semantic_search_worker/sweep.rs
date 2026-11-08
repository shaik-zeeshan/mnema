use super::*;

/// Distinguishes a `load_embedder` failure from a successful-load-but-embed
/// failure when both now run inside the one `spawn_blocking` (M1). The closure
/// returns `Result<(LoadedEmbedder, Vec<per-anchor results>, load time, forward time), LoadError>`: an
/// `Err(LoadError)` means the model never loaded (→ CT3 load-failure accounting:
/// `consecutive_load_failures`, the corrupt-model signal), while an `Ok` with
/// per-anchor `Err`s inside the `Vec` means the model loaded fine and individual
/// anchors failed to embed (→ the existing per-anchor L3 quarantine handling). The
/// two failure modes stay branchable exactly as they were when the load ran inline
/// on the reactor.
pub(super) struct LoadError {
    error: String,
}

/// Run one sweep pass: gate on the installed model, drain up to one batch of
/// anchors newest-first (skipping quarantined poison-pills), embed each on a
/// blocking thread, and store the vectors. Never panics; any error is logged and
/// turned into [`SweepPass::Error`].
pub async fn run_sweep_pass(
    infra: &AppInfraState,
    core: &AppCore,
    state: &mut SweepState,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> SweepPass {
    // The user's selected **Semantic Search Model Tier** (Settings slice #125):
    // the worker reloads the embedder when the provider/model id changes
    // (`embedder_matches`), so a model switch from Settings is picked up live with
    // no restart. A model-tier change to a non-768 dim is preceded by the Settings
    // re-index, which clears the vec0 table; the worker then re-derives every
    // anchor under the new model.
    let settings = effective_semantic_search_settings(core);

    let app_data_dir = core.paths().config.clone();

    // Model-gating: silent no-op when no model is installed (matches the
    // transcription backfill skip). `Err` only on a corrupt marker — treat it as
    // unavailable, never a capture-blocking failure.
    let available = selected_model_available(&app_data_dir, &settings);
    if !available {
        // Drop any previously-loaded embedder so a model that was uninstalled (or a
        // selection turned off) stops holding the session, and the next install
        // reloads cleanly. Reset the CT3 corrupt-model latch and load counter so a
        // fresh (re)install gets a clean set of load attempts.
        state.embedder = None;
        state.clear_load_failures();
        if !state.logged_no_model {
            crate::debug_log::log(
                "semantic index backfill skipped: no Semantic Search Model installed (silent no-op)",
            );
            state.logged_no_model = true;
        }
        return SweepPass::Idle;
    }
    state.logged_no_model = false;

    // Resolve the catalog descriptor (dimension/window/pooling + install path) for
    // the selected model BEFORE the corrupt-model latch check, so the latch can be
    // keyed by the model it was raised for. (Availability said yes but the
    // descriptor vanished — defensive; treat as unavailable for this pass.)
    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        state.embedder = None;
        return SweepPass::Idle;
    };

    // CT3: reconcile the corrupt-model latch against the currently-selected model.
    // The latch is keyed by the `(provider, model_id)` it was raised for: switching
    // to a DIFFERENT (valid) model clears the latch, the load counter, and the
    // stale cached embedder so model B loads cleanly even after model A was flagged
    // corrupt. The `!available` branch above clears it on uninstall/reinstall.
    state.reconcile_selection(&descriptor.provider, &descriptor.model_id);

    // CT3: if the *currently-selected* model has already been signalled corrupt this
    // stretch (N consecutive load failures), idle quietly rather than re-attempting
    // the doomed load every tick. Only the matching identity short-circuits — a
    // different selection was cleared by `reconcile_selection` above.
    if state.corrupt_latch_matches(&descriptor.provider, &descriptor.model_id) {
        return SweepPass::Idle;
    }

    // Peek the backlog before paying for an embedder load: if nothing needs a
    // vector we idle without touching the model.
    let raw_batch = match infra.semantic_search().anchors_missing_vector(SWEEP_BATCH_SIZE).await {
        Ok(batch) => batch,
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "semantic index backfill failed to read anchors missing a vector: {error}"
            ));
            return SweepPass::Error;
        }
    };
    if raw_batch.is_empty() {
        // Caught up: forget every quarantine entry (the backlog is fully drained, so
        // no id is still missing) and the clustered-poison detector. This bulk clear
        // is the *empty-backlog* prune only — continuous capture can keep the backlog
        // non-empty for the whole uptime, so it is NOT a sufficient bound on the map
        // by itself (F18); the per-pass prune below keeps it bounded by the live
        // missing set whether or not the backlog ever reaches zero.
        state.anchor_failures.clear();
        state.clear_transient_stuck();
        return SweepPass::Idle;
    }

    // F18: prune the L3 quarantine map against the live peeked ids each pass, so a
    // sub-cap entry whose anchor stored or was reprocessed (its id retired) cannot
    // leak for the worker's whole uptime when continuous capture keeps the backlog
    // from ever reaching the empty-backlog bulk clear. Quarantined ids that are still
    // missing remain in the map (they must stay filtered out below), so this never
    // resurrects a poison pill. Bounded by the peek window, so this is a small set.
    let live_ids: std::collections::HashSet<i64> =
        raw_batch.iter().map(|anchor| anchor.anchor_id).collect();
    state.prune_anchor_failures(&live_ids);

    // L3 quarantine: drop anchors that have already failed deterministically
    // `MAX_CONSECUTIVE_ANCHOR_FAILURES` times in a row. They are excluded from the
    // batch (so a poison-pill never re-drives the 30s error loop), and the backlog
    // drains *around* them. A reprocess gives the anchor a new id (absent from the
    // map) so it is retried; a restart clears the map entirely.
    let batch: Vec<_> = raw_batch
        .into_iter()
        .filter(|anchor| !state.is_anchor_quarantined(anchor.anchor_id))
        .collect();
    if batch.is_empty() {
        // Every anchor in the peeked window is quarantined. Idle (not Error): there
        // is nothing to retry until a reprocess or restart frees one.
        return SweepPass::Idle;
    }

    // The catalog descriptor (dimension/window/pooling + install path) for the
    // selected model was resolved above (before the corrupt-model latch). The actual
    // embedder load is deferred into the blocking task below (M1): `load_embedder`
    // does heavy synchronous I/O + model init (`fs::read`, `from_mmaped_safetensors`
    // over hundreds of MB, Metal/device init, tokenizer load), so it must never run
    // on the tokio reactor — it is folded into the same `spawn_blocking` as the
    // forward, mirroring the query path.

    // Embed the batch on a blocking thread: BOTH the (conditional) embedder load
    // AND the candle forward are synchronous model work (Metal GPU on macOS /
    // candle-CPU elsewhere) that must stay off the tokio reactor, then store each
    // vector. The cached embedder is moved into the blocking task and back out so it
    // survives across passes (it is shared `&self`-immutable for the embed itself).
    // If the cached embedder is already for this descriptor it is reused; otherwise
    // it is (re)loaded inside the task. The batch is wall-timed to scale the
    // CPU-pacing cooldown that follows.
    //
    // The cached embedder is taken out here so it can be moved into the task; on a
    // load failure (which clears the slot inside the task) it stays `None` and is
    // retried/loaded next pass, matching the pre-M1 behavior where a failed load
    // also left `state.embedder` unset.
    let cached = if embedder_matches(&state.embedder, &descriptor) {
        state.embedder.take()
    } else {
        // A different model (Settings switch) or nothing cached: drop it and (re)load
        // inside the task below.
        state.embedder = None;
        None
    };
    let texts: Vec<(i64, String)> = batch
        .iter()
        .map(|anchor| (anchor.anchor_id, anchor.body_text.clone()))
        .collect();

    // F3: the anchor-id set this pass will attempt, used to drive the clustered-
    // poison detector AFTER the pass completes (a transient-only outcome over the
    // same set, repeated, means the window is stuck). When the detector says we have
    // been stuck on this exact set, embed it ONE ANCHOR AT A TIME so every genuine
    // poison anchor fails in isolation (and is credited toward quarantine), rather
    // than the batched embed misclassifying the whole cluster transient forever.
    let batch_ids: Vec<i64> = batch.iter().map(|anchor| anchor.anchor_id).collect();
    let per_anchor_isolation = state.is_stuck_on(&batch_ids);

    // Take the shared background GPU slot before the forward, so this sweep and the
    // subject-vector sweep never embed concurrently (see [`BACKGROUND_EMBED_SLOT`]).
    // Held until this pass returns; the wait is bounded by one sibling batch. An
    // `Err` here would mean the semaphore was closed, which never happens for a
    // `static` — proceed unpaced rather than dropping the batch.
    let _embed_slot = BACKGROUND_EMBED_SLOT.acquire().await.ok();

    let app_data_dir_for_task = app_data_dir.clone();
    let descriptor_for_task = descriptor.clone();
    // CT2: race the blocking load+embed against the shutdown watch so a quit
    // mid-batch does not wait on a full batch of model load+forward work. If
    // shutdown wins, the blocking task is abandoned (it only loads/computes in
    // memory — dropping it leaves no partial DB state) and the worker stops.
    // `select` polls the shutdown future first, so an already-requested shutdown
    // also wins immediately. The embedder moved into the abandoned task is lost; the
    // worker is exiting anyway.
    let embed_task = core.spawn_blocking(move || {
        // M1: do the heavy `load_embedder` here (off the reactor) when the cached
        // embedder is absent or for a different model. A load failure short-circuits
        // with `LoadError` so the caller runs the CT3 load-failure accounting
        // (consecutive-load-failure counter, corrupt-model signal) — kept DISTINCT
        // from a successful-load-but-per-anchor-embed-failure, which is carried in
        // the returned per-anchor `Vec` for the existing L3 handling.
        let load_started_at = Instant::now();
        let loaded = match cached {
            Some(loaded) => loaded,
            None => match load_embedder(&app_data_dir_for_task, &descriptor_for_task) {
                Ok(loaded) => loaded,
                Err(error) => return Err(LoadError { error }),
            },
        };
        let load_elapsed = load_started_at.elapsed();
        // candle on Metal frees the P-cores by construction, so the retired
        // per-thread background-QoS downclock is gone (ADR 0037); the embed runs at
        // the blocking thread's default QoS. The embedder is `&self`-immutable for
        // embedding, so no `mut` is needed — it is still owned by (and returned out
        // of) this task so it survives across passes.
        //
        // The pacing clock starts HERE, after any (re)load: the cooldown governs the
        // duty cycle of the FORWARD, so it must not see the model load. One pass in
        // every `MAX_EMBEDDER_WARM_LIFETIME` reloads the weights (hundreds of MB off
        // disk into the device) inside this same task, and the per-anchor DB re-checks
        // + the batch write transaction run after it in the caller — timing the whole
        // region would rest 3x all of that, and at
        // `BACKFILL_BATCH_COOLDOWN_MAX = 30s` a recycle pass clamps at the ceiling,
        // which is exactly the "cap is the governor" regime this band was retuned to
        // leave.
        let forward_started_at = Instant::now();
        let out: Vec<(i64, std::result::Result<Vec<f32>, String>)> = if per_anchor_isolation {
            // F3 per-anchor isolation: the batched embed has kept misclassifying this
            // exact window transient over repeated passes (the >=2-anchor poison
            // cluster the whole-batch failure-count heuristic cannot pin to a single
            // anchor). Embed each text in its OWN `embed_texts` call so every genuine
            // poison anchor fails ALONE — the caller then credits each one a per-anchor
            // L3 failure (every failure is isolated by construction here), so the
            // cluster is quarantined and the window slides past it. Slower (one forward
            // per anchor) but only entered when the fast batched path is stuck.
            texts
                .iter()
                .map(|(anchor_id, body)| {
                    let mut result =
                        loaded.embedder.embed_texts(&[body.as_str()], EmbedKind::Document);
                    // `embed_texts` returns exactly one result per input, so a non-empty
                    // input yields one slot; defend against an unexpected empty return.
                    let single: std::result::Result<Vec<f32>, String> = match result.pop() {
                        Some(slot) => slot.map_err(|error| error.to_string()),
                        None => Err(
                            "embed_texts returned no result for a single input".to_string()
                        ),
                    };
                    (*anchor_id, single)
                })
                .collect()
        } else {
            // One batched candle call for the whole batch (vs one `embed_text` per
            // anchor): fewer total forward passes, so the backlog drains sooner.
            // `embed_texts` returns exactly one result per text, in order, with the
            // same overflow-split/single-passthrough/multi-mean-pool semantics as
            // `embed_text`. `bodies` borrows `texts`, so build `out` after it returns.
            let bodies: Vec<&str> = texts.iter().map(|(_, body)| body.as_str()).collect();
            let results = loaded.embedder.embed_texts(&bodies, EmbedKind::Document);
            texts
                .iter()
                .map(|(anchor_id, _)| *anchor_id)
                .zip(results)
                .map(|(anchor_id, result)| (anchor_id, result.map_err(|error| error.to_string())))
                .collect()
        };
        Ok((loaded, out, load_elapsed, forward_started_at.elapsed()))
    });
    let shutdown_changed = shutdown_rx.changed();
    pin_mut!(embed_task, shutdown_changed);
    let load_embed = match select(embed_task, shutdown_changed).await {
        Either::Left((join_result, _)) => match join_result {
            Ok(load_embed) => load_embed,
            Err(error) => {
                crate::debug_log::log_warn(format!(
                    "semantic index backfill embed task panicked/cancelled: {error}"
                ));
                // The embedder (if any) was moved into the failed task; it will be
                // reloaded next pass (the slot is already `None` from the take above).
                //
                // F3 panic-path: a `spawn_blocking` join `Err` is a candle panic (an
                // abort inside the forward), which bypasses the per-anchor result Vec
                // entirely — without crediting anything, a deterministically-panicking
                // input would re-peek the same window and re-panic every 30s forever,
                // never quarantined. So credit each anchor in the peeked batch a
                // per-anchor L3 failure: after `MAX_CONSECUTIVE_ANCHOR_FAILURES` such
                // panics the whole window is quarantined and the sweep slides past it.
                // (A panic cannot be localized to one anchor, so the whole batch is
                // charged; a genuinely-healthy anchor caught in a panicking batch is
                // retried on its next peek and its streak only advances on a repeat.)
                for anchor_id in &batch_ids {
                    state.record_anchor_embed_failure(*anchor_id);
                }
                // The detector is for transient (non-panic) whole-batch faults; a panic
                // is charged per-anchor above, so this is progress toward quarantine —
                // clear the stuck streak so it does not also trip per-anchor isolation.
                state.clear_transient_stuck();
                return SweepPass::Error;
            }
        },
        Either::Right((_, _)) => {
            // Shutdown requested mid-load/embed: abandon the in-flight batch and stop.
            return SweepPass::Shutdown;
        }
    };
    // Branch load-vs-embed exactly as before M1, just sourced from the task result:
    //   - `Err(LoadError)` => CT3 load-failure accounting (distinct failure mode).
    //   - `Ok((loaded, embedded))` => the model loaded OK; per-anchor results carry
    //     any embed failures for the existing L3 handling below.
    let (loaded, embedded, load_elapsed, forward_elapsed) = match load_embed {
        Ok(result) => {
            // A successful load (or a reuse of the cached embedder, which also proves
            // the weights are fine) resets CT3.
            state.clear_load_failures();
            result
        }
        Err(LoadError { error }) => {
            // CT3: availability is presence+marker only — it never validates that the
            // safetensors weights actually load into candle. A truncated / bit-rotted
            // model fails here every 30s forever. Count consecutive load failures;
            // once they hit the cap, surface a "reinstall" signal on the model-status
            // telemetry channel and idle instead of hammering the doomed load. The
            // load now runs on the blocking thread (M1), so this accounting happens
            // after the task returns rather than inline on the reactor — the branching
            // is otherwise identical.
            // The error string is also RETAINED on the state (not just logged) so the
            // debug surface can show why the model will not load.
            let load_failures = state.record_load_failure(&error);
            crate::debug_log::log_warn(format!(
                "semantic index backfill failed to load model '{}/{}' (consecutive load failures: {load_failures}): {error}",
                descriptor.provider, descriptor.model_id
            ));
            if load_failures >= MAX_CONSECUTIVE_LOAD_FAILURES {
                signal_model_appears_corrupt(core, &descriptor, &error);
                // Latch the corrupt signal to THIS selection's identity so a later
                // switch to a different (valid) model is not short-circuited by it.
                state.corrupt_model_signalled =
                    Some((descriptor.provider.clone(), descriptor.model_id.clone()));
                // The slot is already `None` (taken above; the task did not return an
                // embedder on a load failure).
                return SweepPass::Idle;
            }
            return SweepPass::Error;
        }
    };
    // Everything from here to the pacing return is the DB tail (per-anchor
    // re-checks + the batch write transaction) — measured only so `paced_input`
    // can prove it is excluded from the cooldown input.
    let db_tail_started_at = Instant::now();
    // Restore the embedder for the next pass — unless it has been warm past
    // [`MAX_EMBEDDER_WARM_LIFETIME`], in which case drop it here so a long drain
    // (which never goes idle, so `maybe_release_embedder_on_idle_decay` never
    // fires) still returns candle's Metal buffer pool to the OS. The next batch
    // pays one reload. Dropping this `Arc` only frees when the other holders have
    // dropped theirs too — they check the same `loaded_at`, so they recycle within
    // a pass of each other.
    if loaded.is_stale() {
        crate::debug_log::log(format!(
            "semantic index backfill recycled the embedder after {}s warm \
             (bounds candle's Metal buffer pool; reloads on next batch)",
            loaded.loaded_at.elapsed().as_secs()
        ));
    } else {
        state.embedder = Some(loaded);
    }

    // Whole-batch vs per-anchor failure (data-integrity gate): `embed_texts` now
    // isolates a true poison input to its own text (a failing chunk fails only its
    // own text, after a per-chunk retry of any failed sub-batch), so a single
    // failing anchor among healthy siblings is a genuine per-anchor fault. Several
    // anchors failing TOGETHER, by contrast, is the signature of a transient
    // whole-batch fault (e.g. a recurring GPU OOM that fails every chunk even at the
    // single-chunk shape). Crediting each of those a deterministic L3 failure would
    // quarantine up to a whole 16-anchor newest-first window of healthy anchors on
    // one transient fault. So a per-anchor deterministic failure is only credited
    // when an anchor fails in ISOLATION (exactly one embed error in the batch); when
    // more than one anchor fails together the batch is treated as a transient error
    // (back off, quarantine nobody). A genuine single poison anchor among healthy
    // siblings still surfaces alone (the others store), so it still accrues toward
    // quarantine and is eventually excluded — that path is preserved.
    let embed_failure_count = embedded
        .iter()
        .filter(|(_, result)| result.is_err())
        .count();
    // F3: in per-anchor isolation mode each anchor was embedded in its OWN
    // `embed_texts` call, so a failure here IS isolated by construction — credit
    // every failure toward quarantine regardless of how many failed this pass. That
    // is the whole point of dropping to per-anchor embedding: it re-isolates the
    // genuine poison in a >=2-anchor cluster the batched failure-count heuristic
    // (`is_isolated_embed_failure`) keeps misclassifying transient. In the normal
    // batched path the heuristic still applies (one failure among healthy siblings =
    // poison; several together = transient whole-batch fault).
    let credit_failures_per_anchor =
        per_anchor_isolation || is_isolated_embed_failure(embed_failure_count);

    let mut stored = 0u64;
    // Transient errors (DB re-check / store failures, OR a whole-batch embed fault):
    // worth a 30s retry.
    let mut transient_errors = 0u64;
    // Deterministic embed failures (a candle error on this exact input):
    // counted toward per-anchor quarantine, NOT toward the transient retry loop.
    let mut embed_failures = 0u64;
    let mut quarantined = 0u64;
    let mut dimension_skips = 0u64;
    // Vectors that passed the per-anchor re-check, deferred so the whole batch is
    // written in ONE transaction after the loop (one writer-lock acquisition for the
    // batch instead of one per anchor — the churn that starved capture/finalize
    // writes and inflated stop latency).
    let mut to_store: Vec<(i64, Vec<f32>)> = Vec::new();
    for (anchor_id, result) in embedded {
        match result {
            Ok(vector) => {
                // Re-check just before storing so a vector derived from text that
                // was deleted (retention / Delete Recent) or already replaced by a
                // reprocess mid-embed is never inserted as an orphan. The atomic
                // row-conditioned store is the real correctness boundary (M1); this
                // re-check is an early-out optimization.
                match infra.semantic_search().anchor_still_missing_vector(anchor_id).await {
                    // `store_vectors_if_model_matches` is the worker half of the index
                    // authority (the model id stamped on the live vec0 table). If the
                    // embedder reloaded under a new model but the table has not yet
                    // been rebuilt — the non-atomic model-switch window, or
                    // permanently after a failed rebuild — the store is **skipped, not
                    // errored**, so the sweep idles instead of error-looping a doomed
                    // batch every 30s forever, and never writes a vector from one
                    // model's embedding space into another's index. Startup
                    // reconciliation rebuilds the stuck table so the stamp agrees
                    // again and the skipped anchors re-embed.
                    Ok(true) => {
                        // Defer the write: collect this anchor's vector and store the
                        // whole batch in one transaction after the loop (see
                        // `to_store`). The atomic row-conditioned INSERT remains the
                        // correctness boundary; this re-check was always just an
                        // early-out, so deferring the store does not weaken it.
                        to_store.push((anchor_id, vector));
                    }
                    Ok(false) => {
                        // The anchor was deleted or reprocessed mid-embed; skip it
                        // (the new anchor, if any, is picked up next pass). Clear any
                        // streak so a replaced id starts fresh.
                        state.clear_anchor_failures(anchor_id);
                    }
                    Err(error) => {
                        transient_errors += 1;
                        crate::debug_log::log_warn(format!(
                            "semantic index backfill failed to re-check anchor {anchor_id}: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                if !credit_failures_per_anchor {
                    // More than one anchor in this batch failed to embed in the NORMAL
                    // batched path. `embed_texts` already isolates a true poison input
                    // to its own text (a failing chunk fails only its own text, after a
                    // per-chunk retry), so multiple anchors failing TOGETHER is the
                    // signature of a transient whole-batch fault (e.g. a GPU OOM that
                    // recurs even at the single-chunk shape), NOT several independent
                    // poison pills. Treat it as transient — back off and retry the
                    // whole batch later — WITHOUT crediting any anchor a deterministic
                    // L3 failure, so a transient OOM never quarantines a window of
                    // healthy anchors. Do not bump the anchor's streak. (F3: when this
                    // stays stuck over the same window for repeated passes, the next
                    // pass drops to per-anchor isolation, where every failure IS
                    // credited — see `credit_failures_per_anchor`.)
                    transient_errors += 1;
                    crate::debug_log::log_warn(format!(
                        "semantic index backfill batch embed failure ({embed_failure_count} anchors in this batch); treating as transient (no quarantine); anchor {anchor_id} last error: {error}"
                    ));
                    continue;
                }
                // L3: a deterministic embed failure credited to this anchor — either it
                // failed IN ISOLATION in the batched path (the only anchor to fail,
                // siblings stored, so the fault is the input not the batch) OR we are in
                // F3 per-anchor isolation mode where each anchor was embedded alone so
                // its failure is isolated by construction. Bump its consecutive-failure
                // count; quarantine it once it hits the cap so it stops driving the
                // error loop and the window slides past it. This is NOT a transient
                // error.
                embed_failures += 1;
                let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
                if now_quarantined {
                    quarantined += 1;
                    crate::debug_log::log_warn(format!(
                        "semantic index backfill quarantined anchor {anchor_id} after {failures} consecutive embed failures (excluded from the sweep until reprocess/restart); last error: {error}"
                    ));
                } else {
                    crate::debug_log::log_warn(format!(
                        "semantic index backfill failed to embed anchor {anchor_id} (failure {failures}/{MAX_CONSECUTIVE_ANCHOR_FAILURES}): {error}"
                    ));
                }
            }
        }
    }

    // Single-transaction write of every vector that passed its re-check. Folds the
    // per-anchor outcomes back into the same counters the per-anchor store updated:
    // `true` = stored (clear the failure streak), `false` = skipped (dimension
    // mismatch or the anchor vanished mid-embed — not a poison pill).
    if !to_store.is_empty() {
        match infra
            .semantic_search()
            .store_vectors_if_model_matches(&descriptor.model_id, &to_store)
            .await
        {
            Ok(outcomes) => {
                for ((anchor_id, _), was_stored) in to_store.iter().zip(outcomes) {
                    if was_stored {
                        stored += 1;
                        state.clear_anchor_failures(*anchor_id);
                    } else {
                        dimension_skips += 1;
                    }
                }
            }
            Err(error) => {
                // A real DB failure rolled the whole batch back: every anchor in it
                // retries (transient), matching the single-store `Err` arm.
                transient_errors += to_store.len() as u64;
                crate::debug_log::log_warn(format!(
                    "semantic index backfill failed to store {} vector(s) in batch: {error}",
                    to_store.len()
                ));
            }
        }
    }

    if stored > 0 {
        // F3: a store is progress, so the window slid — clear the clustered-poison
        // detector (a later transient-only stall starts a fresh streak, not a stale
        // one inherited from before the drain resumed).
        state.clear_transient_stuck();
        // F6: the backlog count is a logging-only full COUNT(*) on the shared capture
        // pool. Running it every work pass during a fast drain (a work pass can be ~1s)
        // contends with capture, so gate it behind a coarse cadence — log it only once
        // every N work passes. The peek that drives the actual work stays per-pass.
        let backlog = if state.should_count_backlog_this_pass() {
            // Best-effort: a count failure never affects the sweep. Render the
            // outcome as a human-readable token rather than a negative sentinel so
            // the log line is self-documenting (no magic `-1`/`-2` to decode).
            match infra.semantic_search().count_anchors_missing_vector().await {
                Ok(remaining) => remaining.to_string(),
                Err(_) => "count-failed".to_string(),
            }
        } else {
            // Throttled to the coarse cadence above — not counted on this pass.
            "not-counted".to_string()
        };
        crate::debug_log::log(format!(
            "semantic index backfill embedded {stored} anchor(s) (batch={}, embed_failures={embed_failures}, transient_errors={transient_errors}, backlog={backlog})",
            batch.len()
        ));
        // CPU pacing: scale the inter-batch cooldown off this batch's FORWARD time
        // (measured inside the blocking task, so a `MAX_EMBEDDER_WARM_LIFETIME`
        // reload and the DB tail above are excluded) and clamp it, then loop. This
        // replaces the old 0ms yield so a large backfill paces itself instead of
        // sustaining a back-to-back multi-core burn.
        return SweepPass::DidWork(backfill_batch_cooldown(paced_input(
            load_elapsed,
            forward_elapsed,
            db_tail_started_at.elapsed(),
        )));
    }
    if dimension_skips > 0 {
        // Every stored anchor was skipped on a dimension mismatch (a switch in
        // flight, or a table stuck at the old dimension after a failed rebuild).
        // Idle — not Error — so the worker does not burn a 30s retry loop; startup
        // reconciliation (or a successful rebuild) restores agreement. A dimension
        // skip is not a poison-cluster stall, so reset the detector.
        state.clear_transient_stuck();
        crate::debug_log::log(format!(
            "semantic index backfill idled: {dimension_skips} anchor(s) skipped on a vector-dimension mismatch with the live index (awaiting re-index)"
        ));
        return SweepPass::Idle;
    }
    if quarantined > 0 {
        // At least one anchor was newly quarantined this pass: the window is being
        // resolved (a poison pill removed from future peeks), which is progress. This
        // is the F3 success arm: per-anchor isolation re-isolated the cluster and
        // quarantined an anchor, so the next peek slides past it. Reset the detector
        // UNLESS this was an isolation pass that still has OTHER un-quarantined poison
        // (`embed_failures > quarantined`): keep isolating those next pass rather than
        // bouncing back to the batched path that re-stalls on them.
        if !(per_anchor_isolation && embed_failures > quarantined) {
            state.clear_transient_stuck();
        }
        if embed_failures == quarantined {
            // The only non-stored outcome this pass was newly-quarantined poison-pills
            // (nothing left to retry sooner). Idle so the worker does not spin the 30s
            // error loop on anchors it has just decided to skip.
            return SweepPass::Idle;
        }
        // Some anchors quarantined but others are still under the cap: retry the rest.
        return SweepPass::Error;
    }
    if embed_failures > 0 {
        // Embed failures still under the quarantine cap (each credited per-anchor):
        // retry them after a backoff.
        if !per_anchor_isolation {
            // A normal-path isolated single failure (one poison among healthy
            // siblings) is progress toward quarantine, not a transient stall — reset
            // the detector. But when this pass WAS per-anchor isolation (F3), keep the
            // detector set so the NEXT pass re-isolates the same window again and the
            // cluster keeps accruing toward quarantine instead of bouncing back to the
            // batched path that misclassifies it transient. The streak already exceeds
            // the isolation threshold, so leaving it set keeps `is_stuck_on` true; once
            // ALL poison is quarantined the `quarantined > 0` arm above clears it.
            state.clear_transient_stuck();
        }
        return SweepPass::Error;
    }
    if transient_errors > 0 {
        // A whole-batch embed fault or a real DB hiccup with no per-anchor progress:
        // back off and retry the same batch. F3: record this transient-only pass in
        // the clustered-poison detector. If the worker keeps stalling on the SAME
        // anchor-id window (the signature of a >=2-anchor poison cluster the batched
        // failure-count heuristic misclassifies transient), the next pass drops to
        // per-anchor isolation to re-isolate and quarantine the genuine poison so the
        // window finally slides. A single transient blip clears on its own next pass
        // (a different/empty set restarts the streak) and never reaches isolation.
        state.record_transient_pass(batch_ids);
        return SweepPass::Error;
    }
    // The batch was non-empty but every anchor was skipped by the re-check (all
    // deleted/reprocessed mid-embed): treat as work so we loop and pick up the
    // replacements (with a minimal pace), but it is effectively idle if nothing
    // remains. The window changed, so reset the detector.
    state.clear_transient_stuck();
    SweepPass::DidWork(BACKFILL_BATCH_COOLDOWN_MIN)
}

/// Whether an embed failure in a batch should be credited as a deterministic
/// per-anchor (L3) failure, given how many anchors in the SAME batch failed.
///
/// Data-integrity gate: `SemanticSearchEmbedder::embed_texts` isolates a true
/// poison input to its own text (a failing chunk fails only its own text), so a
/// single failing anchor among healthy siblings is a genuine per-anchor fault and
/// is credited toward quarantine. When MORE THAN ONE anchor in the batch fails
/// together, that is the signature of a transient whole-batch fault (e.g. a
/// recurring GPU OOM that fails every chunk even on the per-chunk retry) rather
/// than per-anchor poison — so it is treated as transient (back off) and NO anchor
/// is credited a deterministic failure, which would otherwise quarantine a whole
/// newest-first window of healthy anchors. A genuine single poison anchor still
/// surfaces alone (its healthy batch-mates store), so it still accrues toward
/// quarantine and is eventually excluded.
pub(super) fn is_isolated_embed_failure(embed_failure_count: usize) -> bool {
    embed_failure_count == 1
}

/// CPU-pacing cooldown between backfill batches: the just-finished batch's
/// **forward** wall time — not the whole pass — scaled by
/// [`BACKFILL_BATCH_COOLDOWN_MULTIPLIER`] and clamped to
/// `[BACKFILL_BATCH_COOLDOWN_MIN, BACKFILL_BATCH_COOLDOWN_MAX]`. This mirrors the
/// shape of OCR's Execution Budget governor (`ocr_budget::cooldown_duration`),
/// which clamps `work_ms * multiplier`, so the heavier a batch was the longer the
/// worker yields the cores before the next one — never busy-looping (the floor)
/// and never stalling the backlog (the ceiling).
pub(super) fn backfill_batch_cooldown(batch_duration: Duration) -> Duration {
    let scaled = batch_duration.mul_f64(BACKFILL_BATCH_COOLDOWN_MULTIPLIER);
    scaled.clamp(BACKFILL_BATCH_COOLDOWN_MIN, BACKFILL_BATCH_COOLDOWN_MAX)
}

/// The pacing input for [`backfill_batch_cooldown`] is the FORWARD alone — never
/// the same task's model (re)load and never the caller's DB tail (per-anchor
/// re-checks + the batch write). Neither is the GPU burn the duty cycle governs,
/// and a reload alone eats the entire pacing band (`MAX / multiplier` = 10s), so
/// folding either in would clamp that pass at the ceiling — the "cap is the
/// governor" regime the band was retuned to leave. This seam exists so a test
/// fails if either duration creeps back into the cooldown input.
pub(super) fn paced_input(_load: Duration, forward: Duration, _db_tail: Duration) -> Duration {
    forward
}

/// Surface a "model appears corrupt — reinstall" signal on the model-status
/// telemetry channel (CT3). Reuses the existing download-progress event the
/// Settings UI already listens on: a `Failed` status with a reinstall message
/// triggers a model-status reload in Settings (it reacts to `failed` by reloading
/// status), so the user sees the model flip to "not installed / needs reinstall"
/// rather than the worker silently load-looping forever. Also logged at ERROR.
pub(super) fn signal_model_appears_corrupt(
    core: &AppCore,
    descriptor: &SemanticSearchModelDescriptor,
    error: &str,
) {
    crate::debug_log::log_error(format!(
        "semantic index backfill: model '{}/{}' appears corrupt after {MAX_CONSECUTIVE_LOAD_FAILURES} consecutive load failures — surfacing reinstall signal; last error: {error}",
        descriptor.provider, descriptor.model_id
    ));
    let payload = SemanticSearchModelDownloadProgressDto {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        status: SemanticSearchModelDownloadStatusDto::Failed,
        downloaded_bytes: 0,
        total_bytes: None,
        message: Some(
            "The installed semantic search model appears corrupt and could not be loaded. Reinstall it from Settings.".to_string(),
        ),
    };
    core.host()
        .emit(CoreEvent::SemanticSearchModelDownloadProgress(payload));
}
