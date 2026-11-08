//! One local day as the Journal draws it (`journal-day.ts` + `journal-view.ts`):
//! card slots with frame counts, away-gaps, the pending edge, folded cards, the
//! river (newest first), the spine axis and lanes, and the week strip.

use capture_types::{Activity, ActivityCategory};
use time::UtcOffset;

use crate::insights::stats::{local, start_of_day, DAY_MS, HOUR_MS};

/// `AWAY_GAP_MIN_MS`.
pub const AWAY_GAP_MIN_MS: i64 = 300_000;
/// `SHORT_ACTIVITY_MS`, `SHORT_ACTIVITY_MIN_FRAMES`, `FOLD_NEIGHBOUR_MS`.
pub const SHORT_ACTIVITY_MS: i64 = 120_000;
pub const SHORT_ACTIVITY_MIN_FRAMES: usize = 3;
pub const FOLD_NEIGHBOUR_MS: i64 = 300_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Frames,
    Audio,
    None,
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub activity: Activity,
    pub frames: usize,
    pub media: Media,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    pub start: i64,
    pub end: i64,
    pub audio: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Pending {
    pub active: bool,
    pub since: Option<i64>,
    /// `None` = summarizing; `Some(reason)` = engine unavailable.
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DayModel {
    pub slots: Vec<Slot>,
    pub gaps: Vec<Gap>,
    pub failed: Vec<(i64, i64)>,
    pub pending: Pending,
    pub has_capture: bool,
    pub audio_only: bool,
}

pub struct DayInput<'a> {
    pub activities: &'a [Activity],
    pub frames: &'a [i64],
    pub audio: &'a [(i64, i64)],
    pub failed: &'a [(i64, i64)],
    pub covered_until: Option<i64>,
    pub engine_available: bool,
    pub engine_reason: Option<String>,
    pub day: (i64, i64),
}

/// `buildJournalDay`.
pub fn build(i: DayInput) -> DayModel {
    let (ds, de) = i.day;
    let clamp = |(s, e): (i64, i64)| (s.max(ds), e.min(de));
    let mut audio: Vec<(i64, i64)> = i
        .audio
        .iter()
        .copied()
        .filter(|(s, e)| *s < de && *e > ds)
        .map(clamp)
        .collect();
    audio.sort();
    let audio_in = |s: i64, e: i64| audio.iter().any(|(a, b)| *a < e && *b > s);
    let mut ts: Vec<i64> = i
        .frames
        .iter()
        .copied()
        .filter(|t| *t >= ds && *t < de)
        .collect();
    ts.sort();
    let has_capture = !ts.is_empty() || !audio.is_empty();
    let mut acts: Vec<&Activity> = i
        .activities
        .iter()
        .filter(|a| a.started_at_ms >= ds && a.started_at_ms < de)
        .collect();
    acts.sort_by_key(|a| (a.started_at_ms, a.ended_at_ms, a.id));
    let slots: Vec<Slot> = acts
        .into_iter()
        .map(|a| {
            let frames = ts.partition_point(|t| *t < a.ended_at_ms)
                - ts.partition_point(|t| *t < a.started_at_ms);
            let media = if frames > 0 {
                Media::Frames
            } else if audio_in(a.started_at_ms, a.ended_at_ms) {
                Media::Audio
            } else {
                // shortcut: "expired" (retention removed it) isn't told apart from "none"
                // until the retention verdict is ported; both read as no footage.
                Media::None
            };
            Slot {
                activity: a.clone(),
                frames,
                media,
            }
        })
        .collect();
    let past = |w: i64| ts.iter().any(|t| *t > w) || audio.iter().any(|(_, e)| *e > w);
    let pending_active = match i.covered_until {
        Some(w) => past(w),
        None => has_capture,
    };
    let pending = if pending_active {
        let since = match i.covered_until {
            None => ts
                .first()
                .copied()
                .into_iter()
                .chain(audio.first().map(|a| a.0))
                .min(),
            Some(w) => Some(ds.max(w)),
        };
        let unavailable =
            (!i.engine_available).then(|| i.engine_reason.clone().unwrap_or_default());
        Pending {
            active: true,
            since,
            unavailable,
        }
    } else {
        Pending::default()
    };
    let covered: Vec<i64> = match i.covered_until {
        Some(w) => ts.iter().copied().filter(|t| *t <= w).collect(),
        None => Vec::new(),
    };
    let mut gaps: Vec<Gap> = Vec::new();
    for w in covered.windows(2) {
        let (s, e) = (w[0], w[1]);
        if e - s < AWAY_GAP_MIN_MS {
            continue;
        }
        let (cs, ce) = clamp((s, e));
        let gap = Gap {
            start: cs,
            end: ce,
            audio: audio_in(s, e),
        };
        let woke = slots.iter().any(|sl| {
            sl.activity.started_at_ms <= s
                && s < sl.activity.ended_at_ms.max(sl.activity.started_at_ms + 1)
        });
        match gaps.last_mut() {
            Some(last) if last.end == gap.start && !woke => {
                last.end = gap.end;
                last.audio |= gap.audio;
            }
            _ => gaps.push(gap),
        }
    }
    DayModel {
        slots,
        gaps,
        failed: i
            .failed
            .iter()
            .copied()
            .filter(|(s, e)| *s < de && *e > ds)
            .map(clamp)
            .collect(),
        pending,
        has_capture,
        audio_only: ts.is_empty() && !audio.is_empty(),
    }
}

/// A card as shown: its clipped span, folded sub-lines, `slim` = short with no host.
#[derive(Clone, Debug)]
pub struct Card {
    pub slot: Slot,
    pub start: i64,
    pub end: i64,
    pub subs: Vec<(Slot, i64, i64)>,
    pub slim: bool,
}

/// `foldCards`: chronological.
pub fn fold_cards(slots: &[Slot]) -> Vec<Card> {
    let mut order: Vec<&Slot> = slots.iter().collect();
    order.sort_by(|a, b| {
        a.activity
            .started_at_ms
            .cmp(&b.activity.started_at_ms)
            .then(b.activity.ended_at_ms.cmp(&a.activity.ended_at_ms))
    });
    let mut cards: Vec<Card> = Vec::new();
    let mut reach: Option<usize> = None;
    for slot in order {
        let (s, e) = (slot.activity.started_at_ms, slot.activity.ended_at_ms);
        let start = reach.map_or(s, |r| s.max(cards[r].end));
        if let Some(r) = reach {
            if s < cards[r].end && start >= e {
                cards[r].subs.push((slot.clone(), s, e));
                continue;
            }
        }
        cards.push(Card {
            slot: slot.clone(),
            start,
            end: start.max(e),
            subs: Vec::new(),
            slim: false,
        });
        let ix = cards.len() - 1;
        if reach.is_none_or(|r| cards[ix].end >= cards[r].end) {
            reach = Some(ix);
        }
    }
    let short = |c: &Card| {
        c.end - c.start < SHORT_ACTIVITY_MS
            || (c.slot.media == Media::Frames && c.slot.frames < SHORT_ACTIVITY_MIN_FRAMES)
    };
    let full: Vec<usize> = (0..cards.len()).filter(|i| !short(&cards[*i])).collect();
    let away = |c: &Card, h: &Card| 0.max(c.start - h.end).max(h.start - c.end);
    for ci in 0..cards.len() {
        if !short(&cards[ci]) {
            continue;
        }
        let cat = cards[ci].slot.activity.category;
        let next = full
            .iter()
            .position(|h| cards[*h].start > cards[ci].start)
            .unwrap_or(full.len());
        let mut hosts: Vec<usize> = [
            next.checked_sub(1).map(|i| full[i]),
            full.get(next).copied(),
        ]
        .into_iter()
        .flatten()
        .filter(|h| away(&cards[ci], &cards[*h]) <= FOLD_NEIGHBOUR_MS)
        .collect();
        let same = |h: usize| cat.is_some() && cards[h].slot.activity.category == cat;
        hosts.sort_by(|x, y| {
            same(*y)
                .cmp(&same(*x))
                .then(away(&cards[ci], &cards[*x]).cmp(&away(&cards[ci], &cards[*y])))
                .then((cards[*y].end - cards[*y].start).cmp(&(cards[*x].end - cards[*x].start)))
        });
        match hosts.first() {
            None => cards[ci].slim = true,
            Some(h) => {
                let c = &cards[ci];
                let mut moved = vec![(c.slot.clone(), c.start, c.end)];
                moved.extend(c.subs.clone());
                cards[ci].subs.clear();
                cards[*h].subs.extend(moved);
            }
        }
    }
    let mut shown: Vec<Card> = (0..cards.len())
        .filter(|i| cards[*i].slim || !short(&cards[*i]))
        .map(|i| cards[i].clone())
        .collect();
    for c in &mut shown {
        c.subs.sort_by_key(|s| s.1);
    }
    shown
}

pub enum Row<'a> {
    Card(&'a Card),
    Gap(Gap),
    Failed(i64, i64),
}

impl Row<'_> {
    fn at(&self) -> i64 {
        match self {
            Row::Card(c) => c.start,
            Row::Gap(g) => g.start,
            Row::Failed(s, _) => *s,
        }
    }
}

/// `buildRiver`: newest first; same start, the shorter card above.
pub fn river<'a>(cards: &'a [Card], gaps: &[Gap], failed: &[(i64, i64)]) -> Vec<Row<'a>> {
    let mut rows: Vec<Row> = cards
        .iter()
        .map(Row::Card)
        .chain(gaps.iter().map(|g| Row::Gap(*g)))
        .chain(failed.iter().map(|(s, e)| Row::Failed(*s, *e)))
        .collect();
    let len = |r: &Row| {
        if let Row::Card(c) = r {
            c.end - c.start
        } else {
            0
        }
    };
    rows.sort_by(|a, b| b.at().cmp(&a.at()).then(len(a).cmp(&len(b))));
    rows
}

/// The shown cards over their displayed spans (spine + Day mix).
pub fn shown_activities(cards: &[Card]) -> Vec<Activity> {
    cards
        .iter()
        .map(|c| Activity {
            started_at_ms: c.start,
            ended_at_ms: c.end,
            ..c.slot.activity.clone()
        })
        .collect()
}

/// `spineAxis`: `(start, end, ticks)`, padded 10 min and rounded out to hours.
pub fn spine_axis(first: i64, last: i64, off: UtcOffset) -> (i64, i64, Vec<i64>) {
    let floor = |ms: i64| {
        let t = local(ms, off);
        ms - (t.minute() as i64 * 60_000 + t.second() as i64 * 1000 + t.millisecond() as i64)
    };
    let ceil = |ms: i64| {
        if floor(ms) == ms {
            ms
        } else {
            floor(ms) + HOUR_MS
        }
    };
    let start = floor(first - 600_000);
    let end = (start + HOUR_MS).max(ceil(last + 600_000));
    let len = end - start;
    let step = if len <= 6 * HOUR_MS {
        HOUR_MS
    } else if len <= 12 * HOUR_MS {
        2 * HOUR_MS
    } else {
        3 * HOUR_MS
    };
    let ticks = (0..)
        .map(|k| start + k * step)
        .take_while(|t| *t <= end)
        .collect();
    (start, end, ticks)
}

/// `stackLanes`: `(lane per span, lanes)`.
pub fn stack_lanes(spans: &[(i64, i64)]) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|x, y| {
        spans[*x]
            .0
            .cmp(&spans[*y].0)
            .then(spans[*y].1.cmp(&spans[*x].1))
    });
    let mut ends: Vec<i64> = Vec::new();
    let mut lane = vec![0; spans.len()];
    for i in order {
        let l = ends
            .iter()
            .position(|e| *e <= spans[i].0)
            .unwrap_or(ends.len());
        if l == ends.len() {
            ends.push(spans[i].1);
        } else {
            ends[l] = spans[i].1;
        }
        lane[i] = l;
    }
    (lane, ends.len().max(1))
}

/// `categoryMix`: activity time per category, largest first.
pub fn category_mix(acts: &[Activity]) -> Vec<(Option<ActivityCategory>, i64)> {
    let mut out: Vec<(Option<ActivityCategory>, i64)> = Vec::new();
    for a in acts {
        let ms = (a.ended_at_ms - a.started_at_ms).max(0);
        if ms == 0 {
            continue;
        }
        match out.iter_mut().find(|(c, _)| *c == a.category) {
            Some((_, t)) => *t += ms,
            None => out.push((a.category, ms)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

pub struct WeekDay {
    pub start: i64,
    pub mix: Vec<(Option<ActivityCategory>, i64)>,
    pub activity_ms: i64,
    pub activities: usize,
    pub captured_hours: i64,
    pub future: bool,
}

/// `weekDays`: Mon–Sun holding `day_start`.
pub fn week_days(
    day_start: i64,
    acts: &[Activity],
    hours: &[(i64, i64)],
    now: i64,
    off: UtcOffset,
) -> Vec<WeekDay> {
    let back = local(day_start, off).weekday().number_days_from_monday() as i64;
    let monday = start_of_day(day_start - back * DAY_MS + HOUR_MS * 12, off);
    (0..7)
        .map(|i| {
            let start = start_of_day(monday + i * DAY_MS + HOUR_MS * 12, off);
            let end = start + DAY_MS;
            let own: Vec<Slot> = acts
                .iter()
                .filter(|a| a.started_at_ms >= start && a.started_at_ms < end)
                .map(|a| Slot {
                    activity: a.clone(),
                    frames: 0,
                    media: Media::None,
                })
                .collect();
            let cards = fold_cards(&own);
            let mix = category_mix(&shown_activities(&cards));
            WeekDay {
                start,
                activity_ms: mix.iter().map(|m| m.1).sum(),
                mix,
                activities: cards.len(),
                captured_hours: hours.iter().find(|(d, _)| *d == start).map_or(0, |h| h.1),
                future: start > now,
            }
        })
        .collect()
}

/// `capturedHoursByDay`: `(local day start, hours with capture)`.
pub fn captured_hours(heat: &[capture_types::HeatmapBucket], off: UtcOffset) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    for b in heat.iter().filter(|b| b.intensity_count > 0) {
        let d = start_of_day(b.bucket_start_ms, off);
        match out.iter_mut().find(|(k, _)| *k == d) {
            Some((_, n)) => *n += 1,
            None => out.push((d, 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(id: i64, s: i64, e: i64) -> Activity {
        Activity {
            id,
            title: format!("a{id}"),
            summary: String::new(),
            category: None,
            focus: None,
            started_at_ms: s,
            ended_at_ms: e,
            created_at_ms: e,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn gaps_split_covered_frames_and_short_cards_fold() {
        let m = build(DayInput {
            activities: &[act(1, 0, 600_000), act(2, 600_000, 660_000)],
            frames: &[0, 30_000, 60_000, 1_000_000, 1_060_000],
            audio: &[],
            failed: &[],
            covered_until: Some(1_000_000),
            engine_available: true,
            engine_reason: None,
            day: (0, DAY_MS),
        });
        assert_eq!(
            m.gaps,
            vec![Gap {
                start: 60_000,
                end: 1_000_000,
                audio: false
            }]
        );
        assert!(m.pending.active && m.pending.since == Some(1_000_000));
        let cards = fold_cards(&m.slots);
        assert_eq!(cards.len(), 1, "the 1-minute card folds into its neighbour");
        assert_eq!(cards[0].subs.len(), 1);
        let (lane, lanes) = stack_lanes(&[(0, 10), (5, 20), (10, 30)]);
        assert_eq!((lane, lanes), (vec![0, 1, 0], 2));
    }
}
