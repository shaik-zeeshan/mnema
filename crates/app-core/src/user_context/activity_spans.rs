//! Where a derived Activity lands in time, so new Activities never overlap.
//!
//! An Activity's span is the min..max of its evidence capture times. Two
//! things made spans overlap: the engine splitting a window's items into two
//! Activities whose items interleave (both spans then cover the whole stretch),
//! and continuation widening a neighbour with `MIN`/`MAX` straight across an
//! Activity written in between. Placement runs per draft, oldest first: a
//! draft only claims time no other Activity holds. The one sanctioned overlap
//! is a draft lying wholly inside another's time: it is kept as its own row,
//! nested in that span (the Journal shows it as a sub-line of the bigger card),
//! so its title and summary stay visible.

use app_infra::NewActivityEvidence;

/// `(started_at_ms, ended_at_ms)`.
pub(crate) type Span = (i64, i64);

/// Activities resting on fewer frames than this (and no audio) are stored with
/// a title only: one or two screenshots can't support a paragraph, and the
/// engine writes one anyway.
pub(crate) const MIN_SUMMARY_FRAMES: usize = 3;

/// Too little evidence for a summary paragraph (see [`MIN_SUMMARY_FRAMES`]).
pub(crate) fn too_thin_for_summary(evidence: &[NewActivityEvidence]) -> bool {
    evidence.iter().all(|e| e.subject_type == "frame") && evidence.len() < MIN_SUMMARY_FRAMES
}

/// Where one draft goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Widen the continued neighbour: the time it gains is free.
    Extend(i64),
    /// A new row over the free part of the draft's span — or over its whole
    /// span when it lies wholly inside time another Activity holds (nested).
    Insert(Span),
}

/// Does `a` share time with `held`? A zero-length `held` holds nothing; a
/// zero-length `a` is a point, inside `held` when `held.0 <= a.0 < held.1`.
fn overlaps(a: Span, held: Span) -> bool {
    held.0 < held.1 && a.0 < held.1 && held.0 < a.1.max(a.0 + 1)
}

/// The longest part of `span` no `held` span covers (earliest on a tie), or
/// `None` when it is covered entirely.
fn free_piece(span: Span, held: impl IntoIterator<Item = Span>) -> Option<Span> {
    let mut pieces = vec![span];
    for h in held {
        pieces = pieces
            .into_iter()
            .flat_map(|p| {
                if !overlaps(p, h) {
                    return vec![p];
                }
                [(p.0, h.0), (h.1, p.1)]
                    .into_iter()
                    .filter(|q| q.0 < q.1)
                    .collect()
            })
            .collect();
    }
    pieces.into_iter().min_by_key(|p| (p.0 - p.1, p.0))
}

/// Place a draft spanning `span` (continuing `continues`, if any) against the
/// Activities that already hold time, `claimed` as `(id, span)`.
pub(crate) fn place(span: Span, continues: Option<i64>, claimed: &[(i64, Span)]) -> Placement {
    let others = |id: i64| claimed.iter().filter(move |c| c.0 != id).map(|c| c.1);
    if let Some(&(id, held)) = continues.and_then(|id| claimed.iter().find(|c| c.0 == id)) {
        // Only the time the neighbour would gain has to be free; an overlap it
        // already had (an older row) doesn't block it.
        let gained = [(span.0.min(held.0), held.0), (held.1, span.1.max(held.1))];
        if gained
            .iter()
            .filter(|g| g.0 < g.1)
            .all(|&g| !others(id).any(|o| overlaps(g, o)))
        {
            return Placement::Extend(id);
        }
    }
    Placement::Insert(free_piece(span, claimed.iter().map(|c| c.1)).unwrap_or(span))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn interleaved_window_activities_split_the_time_instead_of_overlapping() {
        // The engine's first Activity holds 6:37–7:13; a second whose items
        // interleave with it runs 6:37–7:10 → fully covered, so it is kept
        // whole, nested inside the first.
        let a = (37 * MIN, 73 * MIN);
        assert_eq!(place(a, None, &[]), Placement::Insert(a));
        let claimed = [(1, a)];
        assert_eq!(
            place((37 * MIN, 70 * MIN), None, &claimed),
            Placement::Insert((37 * MIN, 70 * MIN))
        );
        // Partly covered → only the free tail is claimed.
        assert_eq!(
            place((60 * MIN, 80 * MIN), None, &claimed),
            Placement::Insert((73 * MIN, 80 * MIN))
        );
        // A one-frame Activity inside the first nests; one past it inserts.
        assert_eq!(
            place((40 * MIN, 40 * MIN), None, &claimed),
            Placement::Insert((40 * MIN, 40 * MIN))
        );
        assert_eq!(
            place((73 * MIN, 73 * MIN), None, &claimed),
            Placement::Insert((73 * MIN, 73 * MIN))
        );
    }

    #[test]
    fn continuation_never_widens_across_an_activity_written_in_between() {
        let claimed = [(1, (30 * MIN, 37 * MIN)), (2, (38 * MIN, 42 * MIN))];
        // Continuing 1 with items at 6:45–6:50 would swallow 2 → a new row.
        assert_eq!(
            place((45 * MIN, 50 * MIN), Some(1), &claimed),
            Placement::Insert((45 * MIN, 50 * MIN))
        );
        // Continuing 2 the same way is free time → extend.
        assert_eq!(
            place((45 * MIN, 50 * MIN), Some(2), &claimed),
            Placement::Extend(2)
        );
        // Backwards (backfill): continuing 2 from 6:31 would cover 1 → only
        // the free minute between them becomes a row.
        assert_eq!(
            place((31 * MIN, 40 * MIN), Some(2), &claimed),
            Placement::Insert((37 * MIN, 38 * MIN))
        );
    }

    #[test]
    fn free_piece_keeps_the_longest_gap_between_held_spans() {
        let held = [(10, 20), (30, 35)];
        assert_eq!(free_piece((0, 40), held), Some((0, 10)));
        assert_eq!(free_piece((12, 40), held), Some((20, 30)));
        assert_eq!(free_piece((10, 20), held), None);
    }

    #[test]
    fn summary_needs_three_frames_or_any_audio() {
        let ev = |kind: &str| NewActivityEvidence {
            subject_type: kind.to_string(),
            subject_id: 1,
            captured_at_ms: Some(0),
            is_headline: false,
        };
        assert!(too_thin_for_summary(&[ev("frame"), ev("frame")]));
        assert!(!too_thin_for_summary(&[
            ev("frame"),
            ev("frame"),
            ev("frame")
        ]));
        assert!(!too_thin_for_summary(&[ev("audio_segment")]));
    }
}
