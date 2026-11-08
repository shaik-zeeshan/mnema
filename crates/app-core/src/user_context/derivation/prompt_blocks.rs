use super::*;

/// Total char budget for the USER-AUTHORED CONTEXT block (#107). Statements are
/// included most-recently-edited first (the store lists them by `updated_at_ms`)
/// until adding the next one would exceed this cap; the remaining (older)
/// statements are dropped and the Context surface flags them `inPrompt: false`.
/// ≈1,000 tokens per pass, next to a ≈36k-char activity block.
pub(super) const AUTHORED_CONTEXT_CHAR_CAP: usize = 4_000;

/// The ONE counting rule for the authored-context budget: per statement (in the
/// store's most-recently-edited-first order), its prompt line if it is read, else `None`. Blank
/// statements are skipped; the newest non-blank one is always included; the scan
/// stops at the first statement that would overflow the cap, so everything older
/// is out too. The Context surface's `inPrompt` flag reads this same rule.
pub(crate) fn authored_prompt_lines(authored: &[AuthoredContext]) -> Vec<Option<String>> {
    let mut out = vec![None; authored.len()];
    let mut used = 0usize;
    let mut any = false;
    for (slot, item) in out.iter_mut().zip(authored) {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let line = match item
            .topic
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(topic) => format!("- (topic: {topic}) {text}\n"),
            None => format!("- {text}\n"),
        };
        if used + line.chars().count() > AUTHORED_CONTEXT_CHAR_CAP && any {
            break;
        }
        used += line.chars().count();
        any = true;
        *slot = Some(line);
    }
    out
}

/// Render the USER-AUTHORED CONTEXT prompt block (#107): the user's standing,
/// self-asserted statements, fed to the engine as authoritative context that should
/// steer which Conclusions form and what Subjects matter — clearly labeled as
/// user-asserted, distinct from the derived Activity evidence above. Statements
/// arrive most-recently-edited first and are kept until [`AUTHORED_CONTEXT_CHAR_CAP`] is reached
/// (older ones are dropped). Empty (no trailing block) when the user has authored
/// none, so a default dossier's prompt is unchanged.
pub(super) fn build_authored_context_block(authored: &[AuthoredContext]) -> String {
    let lines: Vec<String> = authored_prompt_lines(authored)
        .into_iter()
        .flatten()
        .collect();
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
pub(super) fn build_dismissed_conclusions_block(dismissals: &[DismissalState]) -> String {
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
pub(super) const KNOWN_SUBJECTS_FALLBACK_LIMIT: i64 = 200;

/// How many lexical-overlap candidate handles the model-free leg contributes to the
/// KNOWN SUBJECTS union per distillation. The leg ranks ALL existing Subjects by
/// shared-word relevance to the recent Activity text and keeps this many best; the
/// real bound on the prompt is [`KNOWN_SUBJECTS_CHAR_CAP`]. Kept modest so a wide,
/// topically-scattered window cannot flood the block with weak lexical matches.
pub(super) const KNOWN_SUBJECTS_LEXICAL_LIMIT: i64 = 20;

/// Total char budget for the KNOWN SUBJECTS block (slices 5/6). Each handle now
/// carries its existing conclusions as indented `id: statement` lines, so the cap
/// budgets those lines too (bumped from 4_000 accordingly). Handles are short but
/// their conclusion lines are not; a big-context cloud LLM effectively sees every
/// candidate, while a small-context local LLM is recency/relevance-bounded by the cap
/// (the accepted worst case — the most-recent/most-relevant handles lead). Calibration-
/// tunable. A subject whose full block (handle + conclusion lines) would overflow
/// degrades to handle-only.
pub(super) const KNOWN_SUBJECTS_CHAR_CAP: usize = 8_000;

/// Per-subject cap on how many existing conclusions are shown under a KNOWN SUBJECTS
/// handle (strongest + most recent, paraphrases skipped — see
/// `list_conclusions_for_subjects`), so a subject's block stops growing with it.
/// Calibration-tunable, like [`KNOWN_SUBJECTS_CHAR_CAP`].
pub(super) const KNOWN_SUBJECTS_CONCLUSIONS_PER_SUBJECT_CAP: i64 = 15;

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
pub(super) fn build_known_subjects_block(
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
pub(super) fn matching_dismissal<'a>(
    dismissals: &'a [DismissalState],
    subject: &str,
    statement: &str,
) -> Option<&'a DismissalState> {
    dismissals.iter().find(|dismissal| {
        dismissal.subject.eq_ignore_ascii_case(subject)
            && dismissal.statement.eq_ignore_ascii_case(statement)
    })
}

/// A Conclusion's evidence-time span: `(formed, last_supported)` = the earliest
/// and latest start of its supporting Activities. Both are dated by the activity
/// the belief came from, never by when distillation ran (a backlog distilled days
/// later keeps its activity dates), and "formed" is when that evidence began.
/// `now` only if none resolve (support_refs is pre-filtered to known ids).
pub(super) fn support_span_ms(
    support_refs: &[i64],
    started_at_by_id: &HashMap<i64, i64>,
    now: i64,
) -> (i64, i64) {
    let starts = || {
        support_refs
            .iter()
            .filter_map(|id| started_at_by_id.get(id).copied())
    };
    (starts().min().unwrap_or(now), starts().max().unwrap_or(now))
}

/// Filter a list of Activity-id refs to the ones present in `valid_ids`,
/// preserving order and dropping duplicates.
pub(super) fn filter_known_refs(
    refs: &[i64],
    valid_ids: &std::collections::HashSet<i64>,
) -> Vec<i64> {
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    refs.iter()
        .copied()
        .filter(|id| valid_ids.contains(id))
        .filter(|id| seen.insert(*id))
        .collect()
}
