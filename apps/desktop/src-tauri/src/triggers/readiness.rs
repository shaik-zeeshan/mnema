//! The Readiness Wait (issue #177, docs/triggers/CONTEXT.md).
//!
//! Between a Meeting Ends firing and the AI run, the trigger waits — bounded —
//! for the processing pipeline to catch up over the meeting window, so the
//! recap runs over transcripts instead of raw audio nobody has processed yet.
//! Pure over an injected probe/sleep/clock, so every semantics choice below is
//! unit-tested without wall-clock time or a DB.
//!
//! Chosen semantics (each is a test):
//! - **Ready**: audio segments overlap the window and no transcription or
//!   diarization job over them is queued/running → proceed (with a coverage
//!   note when the segments only partially cover the window).
//! - **Not recording**: still no overlapping segments once the window has had
//!   [`SEGMENT_SETTLE_MS`] to land (in-flight segments finalize within the
//!   5-minute segment cap, so "no segments yet" at fire time is NOT evidence
//!   of recording-off) → a Skipped Run, never a run over nothing.
//! - **Cap**: after [`READINESS_CAP_MS`] from the firing, proceed with whatever
//!   exists (noting the catch-up may be incomplete) — delivery is "a little
//!   later", never abandoned because a job queue is slow. At the cap with no
//!   segments at all it is still a skip.
//! - A failing probe never wedges the wait: keep polling, and at the cap
//!   proceed (an honest run attempt beats silently dropping the firing).

use std::time::Duration;

/// Poll interval while waiting for the pipeline.
pub(crate) const READINESS_POLL: Duration = Duration::from_secs(30);

/// The bound on the whole wait, from the firing instant (~15 min per ADR 0057).
pub(crate) const READINESS_CAP_MS: i64 = 15 * 60_000;

/// How long after meeting end before "no segments overlap the window" is
/// believed as "Mnema was not recording": the 5-minute Capture Segment
/// Duration cap plus a margin for the finalize/upsert to land.
pub(crate) const SEGMENT_SETTLE_MS: i64 = 6 * 60_000;

/// Slack when judging coverage: segments this close to the window bounds count
/// as covering them (segment boundaries never align with mic-hold instants).
const COVERAGE_SLACK_MS: i64 = 60_000;

pub(crate) const NOT_RECORDING_REASON: &str = "Mnema was not recording during the meeting";

/// One probe read: the audio-segment spans overlapping the meeting window and
/// how many transcription/diarization jobs over them are still pending.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReadinessSnapshot {
    /// `(started_ms, ended_ms)` of every mic/system-audio segment overlapping
    /// the window.
    pub segment_spans_ms: Vec<(i64, i64)>,
    /// Queued/running `audio_transcription` + `speaker_analysis` jobs over
    /// those segments.
    pub pending_jobs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadinessOutcome {
    /// Nothing to work with — record a Skipped Run, never notify.
    Skip { reason: &'static str },
    /// Run the firing; `coverage_note` rides into the firing context when the
    /// recording only partially covers the meeting or the cap cut the wait.
    Proceed { coverage_note: Option<String> },
}

/// `Some(note)` when the overlapping segments leave more than
/// [`COVERAGE_SLACK_MS`] of the window uncovered at either edge.
fn coverage_note(spans: &[(i64, i64)], window: (i64, i64)) -> Option<String> {
    let earliest = spans.iter().map(|span| span.0).min()?;
    let latest = spans.iter().map(|span| span.1).max()?;
    if earliest > window.0 + COVERAGE_SLACK_MS || latest < window.1 - COVERAGE_SLACK_MS {
        Some("The recording covers only part of the meeting window.".to_string())
    } else {
        None
    }
}

/// Wait for the pipeline to be ready over `window = (start_ms, end_ms)`.
///
/// Injected seams: `probe` reads a [`ReadinessSnapshot`], `sleep` waits between
/// polls, `now` is the clock — all so the wait's semantics unit-test.
pub(crate) async fn wait_for_readiness<P, PFut, S, SFut, N>(
    window: (i64, i64),
    fired_at_ms: i64,
    mut probe: P,
    mut sleep: S,
    mut now: N,
) -> ReadinessOutcome
where
    P: FnMut() -> PFut,
    PFut: std::future::Future<Output = Result<ReadinessSnapshot, String>>,
    S: FnMut(Duration) -> SFut,
    SFut: std::future::Future<Output = ()>,
    N: FnMut() -> i64,
{
    loop {
        let now_ms = now();
        let capped = now_ms.saturating_sub(fired_at_ms) >= READINESS_CAP_MS;
        match probe().await {
            Ok(snapshot) => {
                if snapshot.segment_spans_ms.is_empty() {
                    // No recording seen over the window. Believe it only once
                    // in-flight segments have had time to land (or at the cap).
                    if capped || now_ms >= window.1 + SEGMENT_SETTLE_MS {
                        return ReadinessOutcome::Skip {
                            reason: NOT_RECORDING_REASON,
                        };
                    }
                } else if snapshot.pending_jobs == 0 {
                    return ReadinessOutcome::Proceed {
                        coverage_note: coverage_note(&snapshot.segment_spans_ms, window),
                    };
                } else if capped {
                    // The pipeline is still chewing; run anyway, honestly.
                    let mut note = String::from(
                        "Transcription of the meeting may still be catching up, so parts of it may be missing.",
                    );
                    if let Some(coverage) = coverage_note(&snapshot.segment_spans_ms, window) {
                        note.push(' ');
                        note.push_str(&coverage);
                    }
                    return ReadinessOutcome::Proceed {
                        coverage_note: Some(note),
                    };
                }
            }
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "triggers: readiness probe failed (will retry): {error}"
                );
                if capped {
                    return ReadinessOutcome::Proceed {
                        coverage_note: None,
                    };
                }
            }
        }
        sleep(READINESS_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    const MIN_MS: i64 = 60_000;
    /// A 30-minute meeting ending at t=100min; fired at end + 2 min grace.
    const WINDOW: (i64, i64) = (70 * MIN_MS, 100 * MIN_MS);
    const FIRED_AT: i64 = 102 * MIN_MS;

    /// Drives the wait with a scripted probe; every sleep advances the clock by
    /// the poll interval, mirroring the real loop's cadence.
    async fn run_wait(
        snapshots: Vec<Result<ReadinessSnapshot, String>>,
    ) -> (ReadinessOutcome, usize) {
        let clock = Cell::new(FIRED_AT);
        let script = RefCell::new(snapshots);
        let polls = Cell::new(0usize);
        let outcome = wait_for_readiness(
            WINDOW,
            FIRED_AT,
            || {
                polls.set(polls.get() + 1);
                let mut script = script.borrow_mut();
                let next = if script.len() > 1 {
                    script.remove(0)
                } else {
                    // The last snapshot repeats forever.
                    script[0].clone()
                };
                std::future::ready(next)
            },
            |delay| {
                clock.set(clock.get() + delay.as_millis() as i64);
                std::future::ready(())
            },
            || clock.get(),
        )
        .await;
        (outcome, polls.get())
    }

    fn full_recording(pending_jobs: usize) -> ReadinessSnapshot {
        ReadinessSnapshot {
            segment_spans_ms: vec![(WINDOW.0 - MIN_MS, 85 * MIN_MS), (85 * MIN_MS, WINDOW.1)],
            pending_jobs,
        }
    }

    #[test]
    fn recording_off_is_a_skip_only_after_the_settle_window() {
        block_on(async {
            let (outcome, polls) = run_wait(vec![Ok(ReadinessSnapshot::default())]).await;
            assert_eq!(
                outcome,
                ReadinessOutcome::Skip {
                    reason: NOT_RECORDING_REASON
                }
            );
            // Fired at end+2min; settle is end+6min: the first poll (2 min
            // after end) must NOT skip — an in-flight segment may still land.
            // With 30s polls the skip lands on the poll at end+6min.
            assert!(polls > 1, "must not skip on the first poll");
            assert_eq!(polls, 9); // 2min → 6min after end = 8 sleeps + first poll
        });
    }

    #[test]
    fn catch_up_completing_proceeds_without_note_on_full_coverage() {
        block_on(async {
            let (outcome, polls) = run_wait(vec![
                Ok(full_recording(2)),
                Ok(full_recording(1)),
                Ok(full_recording(0)),
            ])
            .await;
            assert_eq!(outcome, ReadinessOutcome::Proceed { coverage_note: None });
            assert_eq!(polls, 3);
        });
    }

    #[test]
    fn partial_recording_proceeds_with_a_coverage_note() {
        block_on(async {
            // Recording started 10 minutes into the meeting.
            let partial = ReadinessSnapshot {
                segment_spans_ms: vec![(WINDOW.0 + 10 * MIN_MS, WINDOW.1)],
                pending_jobs: 0,
            };
            let (outcome, _) = run_wait(vec![Ok(partial)]).await;
            let ReadinessOutcome::Proceed { coverage_note } = outcome else {
                panic!("partial recording must proceed");
            };
            assert_eq!(
                coverage_note.as_deref(),
                Some("The recording covers only part of the meeting window.")
            );
        });
    }

    #[test]
    fn cap_reached_with_pending_jobs_proceeds_with_a_catching_up_note() {
        block_on(async {
            // Jobs never drain: the wait must still end at the 15-min cap.
            let (outcome, polls) = run_wait(vec![Ok(full_recording(3))]).await;
            let ReadinessOutcome::Proceed { coverage_note } = outcome else {
                panic!("the cap proceeds, never abandons");
            };
            assert!(coverage_note
                .expect("cap rides a note")
                .contains("still be catching up"));
            // 15 min of 30s polls plus the first.
            assert_eq!(polls, 31);
        });
    }

    #[test]
    fn probe_errors_poll_until_the_cap_then_proceed() {
        block_on(async {
            let (outcome, polls) = run_wait(vec![Err("db locked".to_string())]).await;
            assert_eq!(outcome, ReadinessOutcome::Proceed { coverage_note: None });
            assert_eq!(polls, 31);
        });
    }

    #[test]
    fn no_segments_at_the_cap_is_still_a_skip() {
        block_on(async {
            // Errors until past the settle deadline, then an honest "nothing
            // recorded" answer: still a Skipped Run, not a run over nothing.
            let mut script: Vec<Result<ReadinessSnapshot, String>> =
                vec![Err("db locked".to_string()); 20];
            script.push(Ok(ReadinessSnapshot::default()));
            let (outcome, _) = run_wait(script).await;
            assert_eq!(
                outcome,
                ReadinessOutcome::Skip {
                    reason: NOT_RECORDING_REASON
                }
            );
        });
    }

    #[test]
    fn coverage_slack_absorbs_segment_boundary_misalignment() {
        // Segments within a minute of the window bounds count as full coverage.
        let spans = vec![(WINDOW.0 + 30_000, WINDOW.1 - 45_000)];
        assert_eq!(coverage_note(&spans, WINDOW), None);
        // Beyond the slack they do not.
        let late = vec![(WINDOW.0 + 2 * MIN_MS, WINDOW.1)];
        assert!(coverage_note(&late, WINDOW).is_some());
    }
}
