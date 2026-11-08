//! Subjects' pure helpers (`subjectsTiers.ts` + `subjectTimeline.ts`): rows per
//! subject, conviction / movement tiers, the lede headline, the 14-day tail,
//! movers, paging and the confidence interpolation the tail reads.

use std::collections::HashMap;

use capture_types::{Conclusion, ConclusionStatus, ConfidenceSnapshot};

use crate::insights::model::Histories;

/// confidence.rs `DISPLAY_FLOOR`: the faded boundary.
pub const DISPLAY_FLOOR: f64 = 0.15;
/// confidence.rs `INITIAL_BASE`: the "just taking shape" ceiling.
pub const INITIAL_BASE: f64 = 0.3;
/// Component-local "strongly held" boundary.
pub const STRONGLY_HELD: f64 = 0.68;
/// Below this many subjects the list is one flat run.
pub const SPARSE_LIMIT: usize = 5;
pub const FORTNIGHT_MS: i64 = 14 * 86_400_000;
/// Conviction-map rows (the rest are in the list).
pub const MAP_PAGE: usize = 12;
pub const MOVING_LIMIT: usize = 8;
/// Rows per tier page.
pub const TIER_PAGE: usize = 10;

pub fn pct(v: f64) -> i64 {
    (v.clamp(0., 1.) * 100.).round() as i64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Conviction,
    Movement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trend {
    Up,
    Steady,
    Down,
}

#[derive(Clone, Debug)]
pub struct SubjectRow {
    pub subject: String,
    /// Confidence-desc.
    pub conclusions: Vec<Conclusion>,
    pub faded: bool,
    pub headline: String,
    pub since_ms: i64,
    pub last_moved_ms: i64,
    pub trend: Trend,
    pub top: f64,
    /// The lead's confidence 14 days ago; `None` = formed this fortnight.
    pub then: Option<f64>,
    /// Per conclusion (lead first): `(faded, points)`.
    pub spark: Vec<(bool, Vec<f64>)>,
}

/// `interpolateConfidence`.
pub fn interpolate(h: &[ConfidenceSnapshot], at: Option<i64>, fallback: f64) -> f64 {
    let (Some(at), Some(first), Some(last)) = (at, h.first(), h.last()) else {
        return fallback;
    };
    if at <= first.snapshot_at_ms {
        return first.confidence;
    }
    if at >= last.snapshot_at_ms {
        return last.confidence;
    }
    for w in h.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if at <= b.snapshot_at_ms {
            let span = b.snapshot_at_ms - a.snapshot_at_ms;
            if span <= 0 {
                return b.confidence;
            }
            let t = (at - a.snapshot_at_ms) as f64 / span as f64;
            return a.confidence + t * (b.confidence - a.confidence);
        }
    }
    last.confidence
}

/// A step this size or more stands alone as a marker; smaller same-direction
/// steps collapse into one run (`MICRO_STEP`).
const MICRO_STEP: f64 = 0.04;

/// `buildMarkers`: `(at, from, to)` confidence moves, oldest first; runs that
/// round to the same whole percent are dropped.
pub fn markers(h: &[ConfidenceSnapshot]) -> Vec<(i64, f64, f64)> {
    let mut out: Vec<(i64, f64, f64)> = Vec::new();
    let mut run: Option<(bool, f64, f64, i64)> = None;
    let flush = |run: &mut Option<(bool, f64, f64, i64)>, out: &mut Vec<(i64, f64, f64)>| {
        if let Some((_, from, to, at)) = run.take() {
            out.push((at, from, to));
        }
    };
    for w in h.windows(2) {
        let (from, to, at) = (w[0].confidence, w[1].confidence, w[1].snapshot_at_ms);
        let d = to - from;
        if d == 0. {
            flush(&mut run, &mut out);
            continue;
        }
        let up = d > 0.;
        if d.abs() >= MICRO_STEP {
            flush(&mut run, &mut out);
            out.push((at, from, to));
        } else if let Some(r) = run.as_mut().filter(|r| r.0 == up) {
            r.2 = to;
            r.3 = at;
        } else {
            flush(&mut run, &mut out);
            run = Some((up, from, to, at));
        }
    }
    flush(&mut run, &mut out);
    out.retain(|(_, a, b)| pct(*a) != pct(*b));
    out
}

/// `fortnightAgo`.
pub fn fortnight_ago(lead: &Conclusion, h: &[ConfidenceSnapshot], now: i64) -> Option<f64> {
    let then = now - FORTNIGHT_MS;
    (lead.formed_at_ms <= then).then(|| interpolate(h, Some(then), lead.confidence))
}

/// `deriveTrend`.
pub fn derive_trend(cs: &[Conclusion], h: Option<&HashMap<i64, Vec<ConfidenceSnapshot>>>) -> Trend {
    let (mut delta, mut n) = (0., 0);
    for c in cs {
        if let Some(p) = h.and_then(|h| h.get(&c.id)).filter(|p| p.len() >= 2) {
            delta += p[p.len() - 1].confidence - p[0].confidence;
            n += 1;
        }
    }
    if n > 0 {
        let avg = delta / n as f64;
        return if avg > 0.04 {
            Trend::Up
        } else if avg < -0.04 {
            Trend::Down
        } else {
            Trend::Steady
        };
    }
    if cs.iter().all(|c| c.status == ConclusionStatus::Faded) {
        Trend::Down
    } else {
        Trend::Steady
    }
}

/// Subjects from the dossier (case-insensitive grouping), in display order:
/// held before faded, then by lead confidence, then name.
pub fn subject_rows(
    conclusions: &[Conclusion],
    histories: &Histories,
    now: i64,
) -> Vec<SubjectRow> {
    let mut groups: Vec<(String, Vec<Conclusion>)> = Vec::new();
    for c in conclusions {
        let key = c.subject.to_lowercase();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, g)) => g.push(c.clone()),
            None => groups.push((key, vec![c.clone()])),
        }
    }
    let mut rows: Vec<SubjectRow> = groups
        .into_iter()
        .map(|(_, cs)| {
            let subject = cs[0].subject.clone();
            let h = histories.get(&subject);
            let mut sorted = cs.clone();
            sorted.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
            let top = sorted[0].clone();
            let spark = sorted
                .iter()
                .map(|c| {
                    let p: Vec<f64> = h
                        .and_then(|h| h.get(&c.id))
                        .map(|v| v.iter().map(|s| s.confidence).collect())
                        .unwrap_or_default();
                    let pts = if p.len() >= 2 {
                        p
                    } else {
                        let v = p.first().copied().unwrap_or(c.confidence);
                        vec![v, v]
                    };
                    (c.status == ConclusionStatus::Faded, pts)
                })
                .collect();
            SubjectRow {
                then: fortnight_ago(&top, h.and_then(|h| h.get(&top.id)).map_or(&[], |v| v), now),
                trend: derive_trend(&cs, h),
                faded: cs.iter().all(|c| c.status == ConclusionStatus::Faded),
                headline: top.statement.clone(),
                since_ms: cs.iter().map(|c| c.formed_at_ms).min().unwrap_or(0),
                last_moved_ms: cs
                    .iter()
                    .map(|c| c.updated_at_ms.max(c.last_supported_at_ms))
                    .max()
                    .unwrap_or(0),
                top: top.confidence,
                subject,
                conclusions: sorted,
                spark,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.faded
            .cmp(&b.faded)
            .then(b.top.total_cmp(&a.top))
            .then(a.subject.cmp(&b.subject))
    });
    rows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Strong,
    Forming,
    Shaping,
    Fading,
}

pub fn conviction_tier(top: f64, faded: bool) -> Tier {
    if faded {
        Tier::Fading
    } else if top >= STRONGLY_HELD {
        Tier::Strong
    } else if top >= INITIAL_BASE {
        Tier::Forming
    } else {
        Tier::Shaping
    }
}

/// One list section: `(id, title, note, faded, rows)`.
pub struct Group<'a> {
    pub id: &'static str,
    pub title: &'static str,
    pub note: &'static str,
    pub faded: bool,
    pub rows: Vec<&'a SubjectRow>,
}

/// `listGroups`: Matches while filtering, one untitled run while sparse, else
/// every tier of the axis (empty ones too).
pub fn list_groups<'a>(
    rows: &'a [SubjectRow],
    axis: Axis,
    matches: Option<Vec<&'a SubjectRow>>,
) -> Vec<Group<'a>> {
    if let Some(m) = matches {
        return if m.is_empty() {
            Vec::new()
        } else {
            vec![Group {
                id: "search",
                title: "Matches",
                note: "",
                faded: false,
                rows: m,
            }]
        };
    }
    if rows.len() < SPARSE_LIMIT {
        return vec![Group {
            id: "all",
            title: "",
            note: "",
            faded: false,
            rows: rows.iter().collect(),
        }];
    }
    type Meta = (
        &'static str,
        &'static str,
        &'static str,
        fn(&SubjectRow) -> bool,
    );
    let conviction: [Meta; 4] = [
        ("strong", "Strongly held", "confidence 68% and up", |r| {
            conviction_tier(r.top, r.faded) == Tier::Strong
        }),
        (
            "forming",
            "Forming",
            "confidence 30–68% · building support",
            |r| conviction_tier(r.top, r.faded) == Tier::Forming,
        ),
        (
            "shaping",
            "Just taking shape",
            "confidence under 30% · early",
            |r| conviction_tier(r.top, r.faded) == Tier::Shaping,
        ),
        ("fading", "Fading", "kept for history", |r| r.faded),
    ];
    let movement: [Meta; 4] = [
        ("warming", "Warming", "gaining support", |r| {
            !r.faded && r.trend == Trend::Up
        }),
        ("steady", "Steady", "holding", |r| {
            !r.faded && r.trend == Trend::Steady
        }),
        ("cooling", "Cooling", "losing support", |r| {
            !r.faded && r.trend == Trend::Down
        }),
        ("fading", "Fading", "kept for history", |r| r.faded),
    ];
    let metas = if axis == Axis::Conviction {
        conviction
    } else {
        movement
    };
    metas
        .into_iter()
        .map(|(id, title, note, test)| {
            let mut rows: Vec<&SubjectRow> = rows.iter().filter(|r| test(r)).collect();
            if axis == Axis::Conviction {
                rows.sort_by(|a, b| b.top.total_cmp(&a.top));
            } else {
                rows.sort_by(|a, b| b.last_moved_ms.cmp(&a.last_moved_ms));
            }
            Group {
                id,
                title,
                note,
                faded: id == "fading",
                rows,
            }
        })
        .collect()
}

/// Subjects per conviction tier (lede names), in display order.
pub fn tier_names(rows: &[SubjectRow]) -> [Vec<String>; 4] {
    let mut out: [Vec<String>; 4] = Default::default();
    for r in rows {
        let i = match conviction_tier(r.top, r.faded) {
            Tier::Strong => 0,
            Tier::Forming => 1,
            Tier::Shaping => 2,
            Tier::Fading => 3,
        };
        out[i].push(r.subject.clone());
    }
    out
}

/// `listNames`: "A and B", or "A, B and 3 more".
pub fn list_names(xs: &[String]) -> String {
    if xs.len() <= 2 {
        xs.join(" and ")
    } else {
        format!("{}, {} and {} more", xs[0], xs[1], xs.len() - 2)
    }
}

/// `subjectsHeadline`: `(lead, de-emphasised fading tail)`.
pub fn headline(names: &[Vec<String>; 4]) -> (String, String) {
    let clause = |xs: &[String], what: &str| {
        if xs.is_empty() {
            String::new()
        } else {
            format!(
                "{} {} {what}",
                list_names(xs),
                if xs.len() > 1 { "are" } else { "is" }
            )
        }
    };
    let mut lead: Vec<String> = [
        clause(&names[0], "strongly held"),
        clause(&names[1], "forming"),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    if lead.is_empty() {
        lead = [clause(&names[2], "just taking shape")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
    }
    let fading = clause(&names[3], "fading into history");
    if lead.is_empty() {
        return (
            if fading.is_empty() {
                String::new()
            } else {
                format!("{fading}.")
            },
            String::new(),
        );
    }
    if fading.is_empty() {
        (format!("{}.", lead.join(". ")), String::new())
    } else {
        (lead.join(". "), format!("— and {fading}."))
    }
}

/// Whole points the lead moved this fortnight; `None` = formed inside it.
pub fn move_pts(r: &SubjectRow) -> Option<i64> {
    r.then.map(|t| pct(r.top) - pct(t))
}

/// `rankMovers`.
pub fn rank_movers<'a>(rows: &[&'a SubjectRow]) -> Vec<&'a SubjectRow> {
    let mut moved: Vec<&SubjectRow> = rows
        .iter()
        .copied()
        .filter(|r| move_pts(r).unwrap_or(0) != 0)
        .collect();
    moved.sort_by_key(|r| std::cmp::Reverse(move_pts(r).unwrap_or(0).abs()));
    let mut formed: Vec<&SubjectRow> = rows.iter().copied().filter(|r| r.then.is_none()).collect();
    formed.sort_by_key(|r| std::cmp::Reverse(r.since_ms));
    moved.into_iter().chain(formed).take(MOVING_LIMIT).collect()
}

/// `rankSubjects` (subjectSearch.ts), simplified: name then statement matches.
// shortcut: the Svelte ranker also scores word prefixes and fuzzy hits; a plain
// case-insensitive contains, names first, until the filter needs that ranking.
pub fn filter<'a>(rows: &'a [SubjectRow], q: &str) -> Vec<&'a SubjectRow> {
    let q = q.trim().to_lowercase();
    let mut by_name: Vec<&SubjectRow> = rows
        .iter()
        .filter(|r| r.subject.to_lowercase().contains(&q))
        .collect();
    let by_text = rows.iter().filter(|r| {
        !r.subject.to_lowercase().contains(&q)
            && r.conclusions
                .iter()
                .any(|c| c.statement.to_lowercase().contains(&q))
    });
    by_name.extend(by_text);
    by_name
}

/// `pageSlice`: `(from, to, page, pages)`.
pub fn page_slice(total: usize, page: usize, size: usize) -> (usize, usize, usize, usize) {
    let per = size.min(total).max(1);
    let pages = total.div_ceil(per).max(1);
    let page = page.min(pages - 1);
    let from = page * per;
    (from, (from + per).min(total), page, pages)
}

/// `pageLabel`: "21–40 of 138", or "All 30".
pub fn page_label(from: usize, to: usize, total: usize) -> String {
    if from == 0 && to == total {
        format!("All {total}")
    } else if to - from == 1 {
        format!("{to} of {total}")
    } else {
        format!("{}–{to} of {total}", from + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headline_names_tiers_and_tails_fading() {
        let n = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let names = [
            n(&["Mnema", "Rust", "A", "B"]),
            n(&["CoreML"]),
            n(&[]),
            n(&["Fitness"]),
        ];
        assert_eq!(
            headline(&names),
            (
                "Mnema, Rust and 2 more are strongly held. CoreML is forming".into(),
                "— and Fitness is fading into history.".into()
            )
        );
        assert_eq!(page_label(20, 40, 138), "21–40 of 138");
        assert_eq!(page_label(0, 30, 30), "All 30");
        assert_eq!(page_slice(23, 9, 10), (20, 23, 2, 3));
    }

    #[test]
    fn interpolation_clamps_and_blends() {
        let s = |c: f64, at: i64| ConfidenceSnapshot {
            confidence: c,
            snapshot_at_ms: at,
        };
        let h = [s(0.2, 0), s(0.6, 100)];
        assert_eq!(interpolate(&h, Some(-5), 0.9), 0.2);
        assert!((interpolate(&h, Some(50), 0.9) - 0.4).abs() < 1e-9);
        assert_eq!(interpolate(&h, None, 0.9), 0.9);
        let m = markers(&[s(0.3, 0), s(0.31, 1), s(0.32, 2), s(0.5, 3), s(0.5, 4)]);
        assert_eq!(
            m,
            vec![(2, 0.3, 0.32), (3, 0.32, 0.5)],
            "micro steps run together"
        );
    }
}
