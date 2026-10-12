//! **Activity continuation** for derivation: show the engine the Activities a
//! neighbouring window already wrote, so a window that carries one of them on
//! (the same video, the same task) extends that row instead of writing a
//! second, differently-titled one.
//!
//! Live windows are only a tick long (2–10 min by tier), so without this an
//! 8-minute video became three Activities. Backfill walks backwards, so the
//! neighbour can sit on either side of the window.

use capture_types::Activity;

use app_infra::UserContextStore;

use super::derivation::{category_label, format_utc_ms, truncate_chars};

/// How close a neighbour must end before (or start after) the window to be
/// offered for continuation. Matches the away-gap the Day spine draws (5 min):
/// past that the user stepped away, and a resumed video is a new Activity.
const CONTINUATION_GAP_MS: i64 = 5 * 60 * 1000;

/// At most this many neighbours are offered, nearest first.
const MAX_NEIGHBOURS: usize = 4;

/// Per-neighbour summary cap inside the prompt block.
const NEIGHBOUR_SUMMARY_CHAR_CAP: usize = 240;

/// The Activities eligible to be continued by window `[start_ms, end_ms]`.
pub(crate) async fn neighbours_for_window(
    store: &UserContextStore,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Activity>, String> {
    let candidates = store
        .list_activities_in_range(
            start_ms.saturating_sub(CONTINUATION_GAP_MS),
            end_ms.saturating_add(CONTINUATION_GAP_MS),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(nearest_neighbours(start_ms, end_ms, candidates))
}

/// The `MAX_NEIGHBOURS` candidates nearest the window (0 when overlapping),
/// returned oldest first.
fn nearest_neighbours(start_ms: i64, end_ms: i64, mut candidates: Vec<Activity>) -> Vec<Activity> {
    let distance = |a: &Activity| {
        if a.ended_at_ms < start_ms {
            start_ms - a.ended_at_ms
        } else if a.started_at_ms > end_ms {
            a.started_at_ms - end_ms
        } else {
            0
        }
    };
    candidates.retain(|a| distance(a) <= CONTINUATION_GAP_MS);
    candidates.sort_by_key(|a| (distance(a), -a.ended_at_ms));
    candidates.truncate(MAX_NEIGHBOURS);
    candidates.sort_by_key(|a| (a.started_at_ms, a.id));
    candidates
}

/// The ONGOING ACTIVITIES prompt block, or empty when there are no neighbours
/// (so a cold window's prompt is unchanged).
pub(crate) fn neighbour_block(neighbours: &[Activity]) -> String {
    if neighbours.is_empty() {
        return String::new();
    }
    let mut block = String::from(
        "\nONGOING ACTIVITIES — already written up from the minutes just before or after this \
window (shown as `id: title — summary [start .. end UTC] category`). Their items are NOT in the \
list above. When items in this window carry one of them on — the same video, page, task or \
conversation continuing — return that Activity with `continues_id` set to its id, its \
evidence_refs set to this window's items for it, and a title and summary rewritten to describe \
the WHOLE Activity from its start, earlier part included — as one Activity, never as \"continued\" \
or \"kept\" doing something. Never write a second Activity for something listed \
here. Leave `continues_id` unset for anything new.\n\n",
    );
    for activity in neighbours {
        block.push_str(&format!(
            "- {}: {} — {} [{} .. {}]{}\n",
            activity.id,
            activity.title.trim(),
            truncate_chars(activity.summary.trim(), NEIGHBOUR_SUMMARY_CHAR_CAP),
            format_utc_ms(activity.started_at_ms),
            format_utc_ms(activity.ended_at_ms),
            activity
                .category
                .map(|c| format!(" {}", category_label(c)))
                .unwrap_or_default(),
        ));
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::ActivityCategory;

    fn activity(id: i64, started_at_ms: i64, ended_at_ms: i64) -> Activity {
        Activity {
            id,
            title: format!("Activity {id}"),
            summary: format!("Summary {id}"),
            category: Some(ActivityCategory::Entertainment),
            focus: None,
            started_at_ms,
            ended_at_ms,
            created_at_ms: 0,
            evidence: Vec::new(),
        }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn neighbours_are_the_nearest_within_the_gap_on_either_side() {
        let window = (100 * MIN, 105 * MIN);
        let picked = nearest_neighbours(
            window.0,
            window.1,
            vec![
                activity(1, 80 * MIN, 94 * MIN),   // ended 6 min before: past the gap
                activity(2, 96 * MIN, 99 * MIN),   // 1 min before
                activity(3, 107 * MIN, 109 * MIN), // 2 min after (backfill side)
                activity(4, 90 * MIN, 97 * MIN),   // 3 min before
                activity(5, 91 * MIN, 96 * MIN),   // 4 min before
                activity(6, 92 * MIN, 95 * MIN),   // 5 min before: 5th nearest, cut
            ],
        );
        let ids: Vec<i64> = picked.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![4, 5, 2, 3], "nearest four, oldest first");
    }

    #[test]
    fn block_is_empty_without_neighbours_and_lists_ids_otherwise() {
        assert_eq!(neighbour_block(&[]), "");
        let block = neighbour_block(&[activity(57, 0, MIN)]);
        assert!(block.contains("continues_id"));
        assert!(block.contains("- 57: Activity 57 — Summary 57 ["));
        assert!(block.trim_end().ends_with("entertainment"));
    }
}
