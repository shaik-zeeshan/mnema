use super::*;

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
pub(super) const CONCLUSION_PREAMBLE_BASE: &str = "You read a list of a single user's recent Activity episodes \
(each with an id, a title, a one or two sentence summary, a capture time, and an optional category) \
and distill open-ended, plain-language Conclusion statements about the user. A Conclusion is a \
natural-language belief such as \"Has been increasingly interested in Apple\", \"Prefers async \
communication\", or \"Checks Slack first thing in the morning\" (a recurring-habit belief under a \
naturally-coined subject like \"Morning routine\") — NOT a fixed subject+attribute+value row and NOT \
a tag. Recurring daily habits and routines are welcome as ordinary Conclusions. For routines about \
focus or attention, state WHEN focused work tends to happen as a neutral observation of the user's \
rhythm — never a judgment about productivity, and never praise or criticism. Each Conclusion is ABOUT a \
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
pub(super) fn conclusion_preamble() -> String {
    format!(
        "{}\n\n{}",
        guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION,
        CONCLUSION_PREAMBLE_BASE
    )
}

/// Most Activities one distillation pass sends (changed + context).
pub(super) const DISTILLATION_ACTIVITY_LIMIT: i64 = 60;

/// Already-distilled recent Activities sent alongside the changed ones, so a
/// pass over two new episodes still sees what the user was just doing.
pub(super) const DISTILLATION_CONTEXT_ACTIVITIES: i64 = 5;

/// Per-summary text cap so one verbose Activity summary cannot dominate the
/// prompt budget.
pub(super) const ACTIVITY_SUMMARY_CHAR_CAP: usize = 600;

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
pub(super) fn build_distillation_prompt(
    activities: &[capture_types::Activity],
    local_offset_ms: Option<i64>,
) -> String {
    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    // Render every time in the user's local wall clock when the offset is known,
    // else fall back to UTC (unchanged legacy behavior).
    let fmt = |ms: i64| match local_offset_ms {
        Some(offset) => format_local_ms(ms, offset),
        None => format_utc_ms(ms),
    };
    let time_note = match local_offset_ms {
        Some(offset) => format!("Times are local ({}).", offset_label(offset / 60_000)),
        None => "All times are UTC.".to_string(),
    };
    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Below is a list of the user's recent Activity episodes, newest first. Each is tagged with \
its numeric Activity id and its start time. {time_note} Distill open-ended Conclusion \
statements about the user and reference the Activity ids that are each Conclusion's supporting (and \
any contradicting) evidence. Return DistilledConclusionBatch.\n\n",
    ));
    prompt.push_str(&format!("Current time: {}\n", fmt(now_ms)));
    prompt.push_str(&format!("Activities ({}):\n\n", activities.len()));

    for activity in activities {
        let category = activity
            .category
            .map(category_label)
            .unwrap_or("uncategorized");
        prompt.push_str(&format!(
            "[id={}] t={} category={category} title={}\n",
            activity.id,
            fmt(activity.started_at_ms),
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
/// Incremental: the pass sends only Activities inserted or extended since the
/// last successful pass (the durable watermark), plus a few recent ones for
/// context, and advances the watermark on success. Nothing changed → no-op with
/// no engine call. Distillation is also a no-op below two Activities: intent /
/// belief signal needs at least a couple of episodes to form a non-flimsy
/// Conclusion.
pub async fn distill_conclusions(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    core: &crate::AppCore,
    subject_vectors: &SubjectVectorStore,
) -> Result<ConclusionDistillationOutcome, String> {
    let input = store
        .distillation_input(
            DISTILLATION_ACTIVITY_LIMIT - DISTILLATION_CONTEXT_ACTIVITIES,
            DISTILLATION_CONTEXT_ACTIVITIES,
        )
        .await
        .map_err(|error| error.to_string())?;
    let (changed_count, context_count) = (input.changed.len(), input.context.len());
    let mut activities = input.changed;
    activities.extend(input.context);
    // The prompt lists Activities newest first.
    activities.sort_by(|a, b| (b.started_at_ms, b.id).cmp(&(a.started_at_ms, a.id)));

    // Nothing changed since the last pass, or nothing to distill from a single
    // Activity.
    let Some(next_watermark) = input.next_watermark.filter(|_| activities.len() >= 2) else {
        return Ok(ConclusionDistillationOutcome {
            upserted: 0,
            gate_drops: Default::default(),
            input_tokens: 0,
            output_tokens: 0,
        });
    };

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
    let started_at_by_id: HashMap<i64, i64> =
        activities.iter().map(|a| (a.id, a.started_at_ms)).collect();

    // Soft guardrail (#96): the engine sees the Sensitive Category Guardrail
    // instruction prepended to the base preamble — it must not form conclusions
    // about health, sexuality, religion, politics, or similar intimate domains.
    // Local-clock offset (Slice 2/3): read once, thread into the prompt (local
    // Activity times) and the Recurrence Digest below. None = never stamped → UTC.
    let offset_min = store
        .local_offset_minutes()
        .await
        .map_err(|e| e.to_string())?;
    let offset_ms = offset_min.map(|m| m * 60_000);
    let preamble = conclusion_preamble();
    let mut prompt = build_distillation_prompt(&activities, offset_ms);
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
        crate::user_context::subject_candidates::select_semantic_subject_candidates(
            core,
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
        crate::user_context::subject_candidates::merge_known_subjects(recency_handles, related);
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
    let shown_ids: std::collections::HashSet<i64> = subject_conclusions
        .iter()
        .map(|(_, id, _, _)| *id)
        .collect();
    prompt.push_str(&build_known_subjects_block(
        &known_subjects,
        &conclusions_by_subject,
    ));
    // RECURRENCE DIGEST (behavioral routines): a compact, self-filtered summary of
    // recurring time-of-day patterns over the trailing window, appended as steering
    // context so the engine can form routine-shaped Conclusions. The digest is empty
    // (no block) when the window shows no recurrence; counted into input_tokens below.
    let digest_now = now_ms();
    let window_start = app_infra::user_context::recurrence_digest_window_start_ms(digest_now);
    let digest_activities = store
        .list_activities_in_range(window_start, digest_now)
        .await
        .map_err(|e| e.to_string())?;
    let digest = app_infra::user_context::build_recurrence_digest(
        &digest_activities,
        offset_ms.unwrap_or(0),
        digest_now,
    );
    if !digest.is_empty() {
        let tz_line = match offset_ms {
            Some(o) => format!("Times below are local ({}).", offset_label(o / 60_000)),
            None => "Times below are UTC.".to_string(),
        };
        prompt.push_str(&format!("\n{tz_line}\n{digest}\n"));
    }
    let input_tokens = estimate_tokens(&preamble) + estimate_tokens(&prompt);

    let batch: DistilledConclusionBatch =
        ai_engine::extract_with_preamble::<DistilledConclusionBatch>(engine, &preamble, &prompt)
            .await
            .map_err(|error| error.to_string())?;

    let output_tokens = serde_json::to_string(&batch)
        .map(|json| estimate_tokens(&json))
        .unwrap_or(0);

    let now = now_ms();
    let mut upserted = 0usize;
    let mut gate_drops = app_infra::DistillationGateDrops::default();
    let mut duplicate_guard = crate::user_context::conclusion_guard::DuplicateGuard::new(core);
    let mut guard_merged = 0usize;
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

        let (formed_at_ms, last_supported_at_ms) =
            support_span_ms(&support_refs, &started_at_by_id, now);

        // Confidence is resolved INSIDE the store's transaction as the
        // reinforcement ratchet (#9/#10): a new Conclusion forms at
        // `initial_confidence(support, contradict)`, an existing one is decayed to
        // now, ratcheted up by fresh support (never reset to a lower window), then
        // dropped by `apply_contradiction` for fresh contradicting evidence. The
        // store needs the resolved support/contradict counts to do this.
        let mut evidence: Vec<NewConclusionEvidence> =
            Vec::with_capacity(support_refs.len() + contradict_refs.len());
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

        // Supersede (ADR 0046): forward the cited id only if we actually showed it
        // — a hallucinated / not-shown id becomes None. The store enforces
        // same-subject / not-pinned / self-block / retire-only-downward strength.
        let supersedes_id = conclusion.supersedes_id.filter(|id| shown_ids.contains(id));
        // Per-belief reinforce (ADR 0043): forward the cited id only if we actually
        // showed it to the model (same drop-hallucinated rule). A belief that cites
        // nothing goes through the duplicate guard first: if it restates an
        // existing conclusion of the same subject, reinforce that one instead of
        // inserting a reworded copy.
        let reinforces_id = match conclusion.reinforces_id.filter(|id| shown_ids.contains(id)) {
            Some(id) => Some(id),
            None => {
                let duplicate = duplicate_guard
                    .find_duplicate(store, subject, statement, supersedes_id)
                    .await?;
                guard_merged += usize::from(duplicate.is_some());
                duplicate
            }
        };

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
                    formed_at_ms,
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

    if guard_merged > 0 {
        crate::debug_log::log(format!(
            "conclusion distillation: duplicate guard turned {guard_merged} new belief(s) into reinforcements"
        ));
    }
    crate::debug_log::log(format!(
        "conclusion distillation sent {} activities ({changed_count} new or extended, \
         {context_count} context)",
        changed_count + context_count
    ));
    // A failed write only means the next pass re-sends these Activities.
    if let Err(error) = store.set_distillation_watermark(next_watermark).await {
        crate::debug_log::log_warn(format!(
            "conclusion distillation could not save its watermark: {error}"
        ));
    }

    Ok(ConclusionDistillationOutcome {
        upserted,
        gate_drops,
        input_tokens,
        output_tokens,
    })
}

/// Current unix time in milliseconds (no `Date.now()`-style nondeterminism).
pub(super) fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
