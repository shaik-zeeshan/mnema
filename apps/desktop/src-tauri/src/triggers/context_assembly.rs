//! Context Assembly (issue #183): the invisible personalization wrap around a
//! Trigger Run.
//!
//! On every firing, [`gather_personalization`] assembles a bounded block of
//! on-device context around the user's Prompt — non-sensitive User-Context
//! conclusions (via the same `recall_context` broker path the sealed tool uses,
//! re-filtered here belt-and-braces) and excerpts of the previous completed
//! runs of the SAME trigger (via the firing ledger's conversation links), with
//! an instruction to note deltas so feedback compounds across runs.
//!
//! **Placement**: the block rides the EPHEMERAL sealed preamble
//! (`ask_ai::build_trigger_preamble`), never the persisted firing question. The
//! question stays the clean firing context + standing instruction (it persists
//! on the turn row and is visible in follow-up history); injecting recalled
//! conclusions or past-run text there would persist personalization data into
//! conversation rows. The static speaker-identity instruction (microphone =
//! the user's own voice — the app has no self person-profile concept) also
//! lives in the preamble, unconditionally.
//!
//! **Past runs are injected directly, not offered as a tool** (ADR 0058, as
//! amended): two truncated excerpts fit comfortably in the preamble, the
//! Sealed Toolbox stays at its three pinned tools, and injection doesn't
//! depend on the model choosing to call anything.
//!
//! Every input is best-effort: a missing or failing piece degrades to absence
//! (the block shrinks, or [`gather_personalization`] returns `None` and the
//! preamble is exactly its pre-#183 shape). Assembly never blocks or fails the
//! run.

use ::app_infra::brokered_access::{
    BrokerRecallContextRequest, BrokerRecalledConclusion, BrokeredCaptureRequest,
    BrokeredCaptureResponse,
};
use ::app_infra::user_context::guardrail::is_sensitive;

use super::run::format_short_local_date;
use super::TriggerDefinition;
use crate::app_infra::AppInfraState;

/// Cap on injected User-Context conclusions (also the `recall_context` limit).
const MAX_CONCLUSIONS: usize = 6;
/// How many previous completed runs of the same trigger are injected.
const MAX_PAST_RUNS: u32 = 2;
/// Per-run excerpt cap, in characters, before truncation.
const PAST_RUN_EXCERPT_CHARS: usize = 1500;

/// One previous completed run of the same trigger: its local date label and the
/// (possibly truncated) report answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PastRun {
    pub date_label: String,
    pub answer: String,
}

/// Truncate to `max_chars` characters on a char boundary, marking the cut.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}\n[… truncated]")
}

/// Build the personalization block from the gathered pieces, or `None` when
/// every piece is absent (no empty scaffolding text confusing the model).
///
/// The sensitive-category guardrail is applied HERE too (belt and braces): the
/// `recall_context` broker path already drops sensitive conclusions, but this
/// seam is the last line before text reaches a possibly-cloud engine, so it
/// re-filters with the same `is_sensitive` (ADR 0030) check.
pub(crate) fn build_personalization_block(
    conclusions: &[BrokerRecalledConclusion],
    past_runs: &[PastRun],
) -> Option<String> {
    let conclusions: Vec<&BrokerRecalledConclusion> = conclusions
        .iter()
        .filter(|c| !is_sensitive(&c.subject, &c.statement))
        .take(MAX_CONCLUSIONS)
        .collect();
    if conclusions.is_empty() && past_runs.is_empty() {
        return None;
    }

    let mut block = String::from(
        "Personalization for this run, assembled on-device (context for YOU — weave it in where \
relevant; never quote it back as its own section):\n",
    );
    if !conclusions.is_empty() {
        block.push_str(
            "What Mnema knows about this user (distilled, non-sensitive beliefs — use them so \
the feedback lands for THIS user, not a generic one):\n",
        );
        for conclusion in &conclusions {
            block.push_str(&format!(
                "- {}: {}\n",
                conclusion.subject, conclusion.statement
            ));
        }
    }
    if !past_runs.is_empty() {
        block.push_str(
            "Previous runs of this same trigger, newest first. Make this run COMPOUND on them: \
explicitly note deltas — what changed since, whether earlier feedback or action items were \
acted on — and do not repeat earlier feedback verbatim.\n",
        );
        for run in past_runs.iter().take(MAX_PAST_RUNS as usize) {
            block.push_str(&format!(
                "Previous run ({}):\n{}\n",
                run.date_label,
                truncate_chars(run.answer.trim(), PAST_RUN_EXCERPT_CHARS)
            ));
        }
    }
    Some(block)
}

/// Gather the assembly inputs for one firing and build the block. Each piece is
/// best-effort; any failure logs at debug level and degrades to absence.
pub(crate) async fn gather_personalization(
    app_handle: &tauri::AppHandle,
    infra: &AppInfraState,
    trigger: &TriggerDefinition,
    offset_minutes: i32,
) -> Option<String> {
    // 1. Non-sensitive User-Context conclusions relevant to the standing
    //    instruction, via the SAME broker path as the sealed `recall_context`
    //    tool (guardrail + relevance + caps included). Activities are left to
    //    the model's own `recall_context` calls.
    let recall = BrokeredCaptureRequest::RecallContext(BrokerRecallContextRequest {
        query: trigger.prompt.clone(),
        limit: Some(MAX_CONCLUSIONS as u32),
        from: None,
        to: None,
    });
    let conclusions =
        match crate::ask_ai::execute_ask_ai_broker_request(app_handle.clone(), recall).await {
            Ok(BrokeredCaptureResponse::RecallContext(response)) => response.conclusions,
            Ok(_) => Vec::new(),
            Err(error) => {
                tauri_plugin_log::log::debug!(
                    "triggers: context assembly recall_context unavailable for '{}': {error}",
                    trigger.id
                );
                Vec::new()
            }
        };

    // 2. The previous completed runs of this trigger: ledger link → the run's
    //    report (the first completed turn of its conversation).
    let mut past_runs: Vec<PastRun> = Vec::new();
    let firings = infra
        .trigger_firings()
        .recent_completed_firings(&trigger.id, MAX_PAST_RUNS)
        .await
        .unwrap_or_default();
    for firing in firings {
        let Some(conversation_id) = firing.conversation_id.as_deref() else {
            continue;
        };
        let conversation = infra
            .conversation()
            .get_conversation(conversation_id)
            .await
            .ok()
            .flatten();
        let Some(conversation) = conversation else {
            continue;
        };
        // The run's report is the conversation's first completed turn (later
        // turns are the user's follow-up chat, not the document).
        let Some(answer) = conversation
            .turns
            .iter()
            .find(|turn| turn.phase == "done" && !turn.answer.trim().is_empty())
            .map(|turn| turn.answer.clone())
        else {
            continue;
        };
        past_runs.push(PastRun {
            date_label: format_short_local_date(firing.fired_at_ms, offset_minutes),
            answer,
        });
    }

    build_personalization_block(&conclusions, &past_runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conclusion(subject: &str, statement: &str) -> BrokerRecalledConclusion {
        BrokerRecalledConclusion {
            subject: subject.to_string(),
            statement: statement.to_string(),
            confidence: 0.9,
            status: "visible".to_string(),
        }
    }

    #[test]
    fn sensitive_conclusions_never_pass_the_assembly_seam() {
        // Belt-and-braces (ADR 0030): even if a sensitive conclusion reached
        // this seam (the broker path should already have dropped it), the
        // assembly re-filters it out.
        let conclusions = vec![
            conclusion("Rust", "Is in a Rust learning phase"),
            conclusion("mood", "appears depressed lately"),
            conclusion("family planning", "is probably pregnant"),
            conclusion("identity", "is gay"),
            conclusion("design", "Cares a lot about clean design"),
        ];
        let block = build_personalization_block(&conclusions, &[]).expect("block");
        assert!(block.contains("Rust learning phase"));
        assert!(block.contains("clean design"));
        for leaked in ["depressed", "pregnant", "is gay"] {
            assert!(
                !block.contains(leaked),
                "sensitive conclusion leaked through the assembly seam: {leaked}"
            );
        }

        // ONLY sensitive conclusions and no past runs → no block at all.
        let all_sensitive = vec![conclusion("mood", "appears depressed lately")];
        assert_eq!(build_personalization_block(&all_sensitive, &[]), None);
    }

    #[test]
    fn conclusions_are_capped() {
        let conclusions: Vec<BrokerRecalledConclusion> = (0..20)
            .map(|i| conclusion(&format!("subject{i}"), &format!("statement number {i}")))
            .collect();
        let block = build_personalization_block(&conclusions, &[]).expect("block");
        assert!(block.contains("subject5"));
        assert!(!block.contains("subject6"), "conclusion cap not applied");
    }

    #[test]
    fn past_runs_inject_answer_content_and_the_delta_instruction() {
        let past_runs = vec![
            PastRun {
                date_label: "Fri Jul 24".to_string(),
                answer: "## Summary\nYou interrupted twice; try pausing.".to_string(),
            },
            PastRun {
                date_label: "Thu Jul 23".to_string(),
                answer: "## Summary\nAction items were unowned.".to_string(),
            },
        ];
        let block = build_personalization_block(&[], &past_runs).expect("block");
        // Both previous reports ride along, labeled and newest first.
        assert!(block.contains("Previous run (Fri Jul 24):"));
        assert!(block.contains("You interrupted twice; try pausing."));
        assert!(block.contains("Previous run (Thu Jul 23):"));
        assert!(block.contains("Action items were unowned."));
        assert!(
            block.find("Fri Jul 24").expect("newest") < block.find("Thu Jul 23").expect("older")
        );
        // The compounding instruction is present.
        assert!(block.contains("COMPOUND"));
        assert!(block.contains("note deltas"));
        // No conclusions → no conclusions scaffolding.
        assert!(!block.contains("What Mnema knows about this user"));
    }

    #[test]
    fn past_run_excerpts_are_truncated() {
        let long = "x".repeat(PAST_RUN_EXCERPT_CHARS * 2);
        let block = build_personalization_block(
            &[],
            &[PastRun {
                date_label: "Fri Jul 24".to_string(),
                answer: long,
            }],
        )
        .expect("block");
        assert!(block.contains("[… truncated]"));
        assert!(block.len() < PAST_RUN_EXCERPT_CHARS * 2);
    }

    #[test]
    fn all_pieces_absent_degrades_to_none() {
        // Nothing gathered → no scaffolding text at all; the sealed preamble
        // stays exactly its pre-personalization shape.
        assert_eq!(build_personalization_block(&[], &[]), None);
    }
}
