//! The Digest prompt: Activity lines, rollups, tiers, and narrative/headline
//! normalization.

use super::*;

/// Per-activity summary cap inside the Digest prompt, so one verbose Activity
/// cannot dominate the budget. Mirrors `derivation.rs`'s per-item-cap approach
/// (`ACTIVITY_SUMMARY_CHAR_CAP` there is module-private, so the value is
/// restated here); kept smaller because a month range can hold many Activities.
pub(super) const DIGEST_SUMMARY_CHAR_CAP: usize = 240;

/// Total char budget for the rendered Activity lines. Mirrors how
/// `derivation.rs` bounds its prompt inputs (per-item caps + a bounded item
/// count via `MAX_ITEMS`); expressed here as a char budget because the input is
/// a whole calendar range rather than a capture window. ~6k estimated tokens.
///
/// We do NOT drop the oldest Activities when over budget. Instead the prompt
/// **degrades by recency** ([`build_digest_prompt`]): the newest episodes stay
/// at full detail, earlier ones collapse to a one-line compact form, and the
/// oldest are summarized one line per local day (a cached day-digest narrative
/// when one exists, else computed stats). The whole range is always
/// represented, just at decreasing detail going back in time.
pub(super) const DIGEST_PROMPT_CHAR_CAP: usize = 24_000;

/// Sample-title char cap inside a computed rollup line: long titles are
/// shortened so two of them fit on one day line.
pub(super) const ROLLUP_TITLE_CHAR_CAP: usize = 40;

/// How many top categories a computed rollup line names before stopping.
pub(super) const ROLLUP_TOP_CATEGORIES: usize = 3;

/// How many sample titles a computed rollup line folds in.
pub(super) const ROLLUP_SAMPLE_TITLES: usize = 2;

/// The structured result the engine returns for one Digest pass. Flat (two
/// string fields), so the schema is `$ref`/`$defs`-free without `#[schemars(inline)]`;
/// the reference-free guard test below keeps it that way.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DigestNarrative {
    /// At most 2 short sentences of second-person plain prose.
    pub narrative: String,
    /// Full-sentence headline above the prose, with no dash in it. Defaulted
    /// (same stance as `derivation.rs`'s optional fields) so an engine that
    /// omits it still yields a narrative-only digest instead of an extraction
    /// failure; [`joined_headline`] turns empty/whitespace into `None`.
    #[serde(default)]
    pub headline: String,
    /// Optional clause continuing the headline ("until Thursday's calls took
    /// the whole day back"); empty when nothing turned. Stored joined onto the
    /// headline as `headline — turn` (no column of its own).
    #[serde(default)]
    pub turn: String,
}

/// snake_case label for a [`FocusLevel`] in the Digest prompt.
pub(super) fn focus_label(focus: FocusLevel) -> &'static str {
    match focus {
        FocusLevel::Deep => "deep",
        FocusLevel::Mixed => "mixed",
        FocusLevel::Distracted => "distracted",
    }
}

pub(super) const DAY_MS: i64 = 86_400_000;

/// Recover the local UTC-offset (in ms) from the frontend-computed range start.
///
/// The frontend computes the half-open digest window on the LOCAL calendar, so
/// `range_start_ms` is a local midnight: `(range_start_ms + offset) % day == 0`.
/// That pins the offset modulo 24h; the representative is chosen in
/// `(-10h, +14h]`, which covers every real-world offset except UTC−11/−12
/// (those label days one weekday late — a cosmetic prompt-label skew only,
/// never a correctness issue, since the label feeds nothing but the narrative).
pub(super) fn recover_local_offset_ms(range_start_ms: i64) -> i64 {
    let offset = (-range_start_ms).rem_euclid(DAY_MS); // in [0, 24h)
    if offset > 14 * 3_600_000 {
        offset - DAY_MS
    } else {
        offset
    }
}

/// Local-midnight (UTC ms) of the local day containing `at_ms`, using the
/// offset recovered from the range start. The result matches the
/// `range_start_ms` a DAY-kind [`StoredDigest`] is keyed on (the frontend
/// computes those starts the same way), so it is the join key for reusing a
/// cached day-digest narrative in a rollup line.
///
/// Same `div_euclid`/`rem_euclid`-flavoured arithmetic the [`day_label`] /
/// [`recover_local_offset_ms`] pair uses: shift into local time, floor to the
/// day grid, shift back. Saturating adds keep hostile timestamps from panicking.
pub(super) fn local_day_start_ms(at_ms: i64, local_offset_ms: i64) -> i64 {
    let shifted = at_ms.saturating_add(local_offset_ms);
    let local_midnight = shifted.div_euclid(DAY_MS).saturating_mul(DAY_MS);
    local_midnight.saturating_sub(local_offset_ms)
}

/// Compact local day label ("Tue Jun 9") for one Activity start time, using the
/// offset recovered from the range start. Falls back to the epoch date on an
/// out-of-range timestamp (never panics on hostile data).
pub(super) fn day_label(at_ms: i64, local_offset_ms: i64) -> String {
    let shifted_seconds = (at_ms.saturating_add(local_offset_ms)).div_euclid(1_000);
    let date = OffsetDateTime::from_unix_timestamp(shifted_seconds)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .date();
    let weekday = match date.weekday() {
        time::Weekday::Monday => "Mon",
        time::Weekday::Tuesday => "Tue",
        time::Weekday::Wednesday => "Wed",
        time::Weekday::Thursday => "Thu",
        time::Weekday::Friday => "Fri",
        time::Weekday::Saturday => "Sat",
        time::Weekday::Sunday => "Sun",
    };
    let month = match date.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    format!("{weekday} {month} {}", date.day())
}

/// Compact duration label for one Activity span: "<1m", "45m", "2h 05m".
pub(super) fn duration_label(started_at_ms: i64, ended_at_ms: i64) -> String {
    let minutes = (ended_at_ms.saturating_sub(started_at_ms)).max(0) / 60_000;
    if minutes < 1 {
        "<1m".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    }
}

/// Local wall-clock `HH:MM` of `at_ms`, using the offset recovered from the
/// range start.
pub(super) fn clock_label(at_ms: i64, local_offset_ms: i64) -> String {
    let minutes = at_ms.saturating_add(local_offset_ms).rem_euclid(DAY_MS) / 60_000;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Local `HH:MM–HH:MM` span of one Activity.
pub(super) fn clock_span(activity: &Activity, local_offset_ms: i64) -> String {
    format!(
        "{}–{}",
        clock_label(activity.started_at_ms, local_offset_ms),
        clock_label(activity.ended_at_ms, local_offset_ms)
    )
}

/// Render one Activity as a single compact prompt line:
/// `- [Tue Jun 9 | 09:20–10:50 | 1h 30m | coding | deep | in Zed, Safari] Title — summary`
/// (category/focus/app segments are omitted when unset). `apps` are the
/// Activity's top evidence apps (`list_activity_meta_in_range`): app names only.
pub(super) fn format_activity_line(
    activity: &Activity,
    local_offset_ms: i64,
    apps: &[String],
) -> String {
    let mut meta = format!(
        "{} | {} | {}",
        day_label(activity.started_at_ms, local_offset_ms),
        clock_span(activity, local_offset_ms),
        duration_label(activity.started_at_ms, activity.ended_at_ms)
    );
    if let Some(category) = activity.category {
        meta.push_str(" | ");
        meta.push_str(category_label(category));
    }
    if let Some(focus) = activity.focus {
        meta.push_str(" | ");
        meta.push_str(focus_label(focus));
    }
    if !apps.is_empty() {
        meta.push_str(" | in ");
        meta.push_str(&apps.join(", "));
    }
    let title = activity.title.trim();
    let summary = truncate_chars(activity.summary.trim(), DIGEST_SUMMARY_CHAR_CAP);
    if summary.is_empty() {
        format!("- [{meta}] {title}\n")
    } else {
        format!("- [{meta}] {title} — {summary}\n")
    }
}

/// Render one Activity as the **compact** middle-tier line:
/// `- [Tue Jun 9 | 09:20–10:50 | 1h 30m] Title` — day + clock span + duration +
/// title only, dropping the summary, category, and focus the full
/// [`format_activity_line`] carries. Used for earlier episodes once the
/// full-detail prompt would blow the budget.
pub(super) fn format_activity_line_compact(activity: &Activity, local_offset_ms: i64) -> String {
    format!(
        "- [{} | {} | {}] {}\n",
        day_label(activity.started_at_ms, local_offset_ms),
        clock_span(activity, local_offset_ms),
        duration_label(activity.started_at_ms, activity.ended_at_ms),
        activity.title.trim(),
    )
}

/// Render the **rollup** bottom-tier line for one local day's worth of the
/// oldest activities. `day_narrative` is the cached day-digest prose when one
/// exists for that local day (the **hybrid** path); when present its prose is
/// used verbatim and no stats are computed.
///
/// The **computed** fallback names: the activity count, the summed duration
/// (via [`duration_label`] over `0..total_ms`), the top
/// [`ROLLUP_TOP_CATEGORIES`] categories by summed duration (omitted when no
/// activity in the day carries a category), and up to [`ROLLUP_SAMPLE_TITLES`]
/// sample titles — the longest-duration episodes, short-capped at
/// [`ROLLUP_TITLE_CHAR_CAP`] (omitted when none have a non-empty title). Both
/// optional clauses collapse cleanly so a label-free day reads as just
/// `- [Tue Jun 9] 3 activities, 2h 10m`.
pub(super) fn format_rollup_line(
    day_activities: &[&Activity],
    local_offset_ms: i64,
    day_narrative: Option<&str>,
) -> String {
    // The day label is taken from the first (chronologically earliest)
    // activity; they all share the same local day by construction.
    let label = day_activities
        .first()
        .map(|a| day_label(a.started_at_ms, local_offset_ms))
        .unwrap_or_else(|| day_label(0, local_offset_ms));

    // Hybrid path: reuse the day digest's own prose verbatim.
    if let Some(narrative) = day_narrative {
        let narrative = narrative.trim();
        if !narrative.is_empty() {
            return format!("- [{label}] {narrative}\n");
        }
    }

    // Computed path: count + summed duration + top categories + sample titles.
    let count = day_activities.len();
    let total_ms: i64 = day_activities
        .iter()
        .map(|a| a.ended_at_ms.saturating_sub(a.started_at_ms).max(0))
        .sum();
    let plural = if count == 1 { "activity" } else { "activities" };
    let mut line = format!(
        "- [{label}] {count} {plural}, {}",
        duration_label(0, total_ms)
    );

    // Top categories by summed duration. A stable insertion order keeps ties
    // deterministic (no `Date::now`/random tie-break).
    let mut category_ms: Vec<(ActivityCategory, i64)> = Vec::new();
    for activity in day_activities {
        let Some(category) = activity.category else {
            continue;
        };
        let span = activity
            .ended_at_ms
            .saturating_sub(activity.started_at_ms)
            .max(0);
        match category_ms.iter_mut().find(|(c, _)| *c == category) {
            Some((_, ms)) => *ms = ms.saturating_add(span),
            None => category_ms.push((category, span)),
        }
    }
    if !category_ms.is_empty() {
        // Stable sort by descending duration preserves first-seen order on ties.
        category_ms.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = category_ms
            .iter()
            .take(ROLLUP_TOP_CATEGORIES)
            .map(|(category, ms)| {
                format!("{} ({})", category_label(*category), duration_label(0, *ms))
            })
            .collect();
        line.push_str(" — top categories: ");
        line.push_str(&top.join(", "));
    }

    // Sample titles: the longest-duration episodes, short-capped.
    let mut by_duration: Vec<&&Activity> = day_activities.iter().collect();
    by_duration.sort_by(|a, b| {
        let da = a.ended_at_ms.saturating_sub(a.started_at_ms).max(0);
        let db = b.ended_at_ms.saturating_sub(b.started_at_ms).max(0);
        db.cmp(&da)
    });
    let samples: Vec<String> = by_duration
        .iter()
        .filter_map(|a| {
            let title = a.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(format!(
                    "\"{}\"",
                    truncate_chars(title, ROLLUP_TITLE_CHAR_CAP)
                ))
            }
        })
        .take(ROLLUP_SAMPLE_TITLES)
        .collect();
    if !samples.is_empty() {
        line.push_str("; e.g. ");
        line.push_str(&samples.join(", "));
    }

    line.push('\n');
    line
}

/// Detail tier each Activity is currently rendered at while degrading the
/// prompt to fit the budget. `Full` is the richest, `Rollup` the tersest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tier {
    Full,
    Compact,
    Rollup,
}

/// Build the Digest prompt: a header naming the range, then the Activities in
/// chronological order (newest LAST — chronology reads better for a narrative).
///
/// When the full-detail lines would exceed [`DIGEST_PROMPT_CHAR_CAP`] the prompt
/// **degrades by recency** rather than dropping the oldest: the most recent
/// episodes stay [`Tier::Full`], earlier ones collapse to a one-line
/// [`Tier::Compact`] form, and the oldest are summarized one line per local day
/// ([`Tier::Rollup`]) — using a cached day-digest narrative from `day_digests`
/// when one exists for that day (the hybrid path), else computed stats. The
/// whole range is always represented, so the engine never mistakes a degraded
/// range for a partial one; a tier-explanation note says so when degradation
/// happened.
///
/// `day_digests` are DAY-kind cached digests overlapping the range; the slice is
/// empty for the "day" range kind (a day never reuses its own digest as input).
/// They are keyed by their `range_start_ms` — a local midnight in UTC ms — which
/// [`local_day_start_ms`] reproduces from an activity's `started_at_ms`.
pub(super) fn build_digest_prompt(
    range_kind: &str,
    range_start_ms: i64,
    range_end_ms: i64,
    now_ms: i64,
    activities: &[Activity],
    day_digests: &[StoredDigest],
    apps: &HashMap<i64, Vec<String>>,
) -> String {
    let local_offset_ms = recover_local_offset_ms(range_start_ms);

    // Local-day-start (UTC ms) → cached day-digest narrative, for the hybrid
    // rollup path. Keyed exactly as `local_day_start_ms` computes a day key.
    let day_narratives: HashMap<i64, &str> = day_digests
        .iter()
        .map(|d| (d.range_start_ms, d.narrative.as_str()))
        .collect();

    // `activities` arrives chronological (oldest first) from the store query.
    // Index 0 is the oldest; degradation eats from the front.
    let n = activities.len();

    // Each activity starts at Full; we recompute the running total as we degrade.
    let full_lines: Vec<String> = activities
        .iter()
        .map(|a| format_activity_line(a, local_offset_ms, apps.get(&a.id).map_or(&[], |v| v)))
        .collect();
    let compact_lines: Vec<String> = activities
        .iter()
        .map(|a| format_activity_line_compact(a, local_offset_ms))
        .collect();
    let line_cost = |s: &str| s.chars().count();

    let mut tiers = vec![Tier::Full; n];
    let mut total: usize = full_lines.iter().map(|s| line_cost(s)).sum();
    let mut degraded = false;

    // The local-day key of each activity, used to group the rollup tier.
    let day_keys: Vec<i64> = activities
        .iter()
        .map(|a| local_day_start_ms(a.started_at_ms, local_offset_ms))
        .collect();

    // Step 1 (cap-fit): if everything fits at Full, emit unchanged. Else degrade.
    if total > DIGEST_PROMPT_CHAR_CAP {
        degraded = true;

        // Step 2: Full → Compact from oldest to newest, one at a time, stopping
        // as soon as we are under the cap (leaves newest=Full, oldest=Compact).
        for i in 0..n {
            if total <= DIGEST_PROMPT_CHAR_CAP {
                break;
            }
            tiers[i] = Tier::Compact;
            total = total + line_cost(&compact_lines[i]) - line_cost(&full_lines[i]);
        }

        // Step 3: still over after all-Compact → roll up whole oldest DAYS.
        // Group the currently-Compact prefix by local day; move the oldest such
        // day wholesale into a single Rollup line, recomputing, until under the
        // cap OR only one compact day remains. Full-tier activities are never
        // rolled up.
        if total > DIGEST_PROMPT_CHAR_CAP {
            // Build the rollup-line cost lazily per day as we collapse it.
            loop {
                // Indices still in Compact, contiguous from the oldest end.
                let compact_idx: Vec<usize> =
                    (0..n).filter(|&i| tiers[i] == Tier::Compact).collect();
                if compact_idx.is_empty() {
                    break;
                }
                // Distinct local days still represented in the Compact band.
                let mut compact_days: Vec<i64> = Vec::new();
                for &i in &compact_idx {
                    if !compact_days.contains(&day_keys[i]) {
                        compact_days.push(day_keys[i]);
                    }
                }
                if total <= DIGEST_PROMPT_CHAR_CAP || compact_days.len() <= 1 {
                    break;
                }

                // Oldest compact day = the first distinct day key (chronological).
                let target_day = compact_days[0];
                let members: Vec<&Activity> = (0..n)
                    .filter(|&i| tiers[i] == Tier::Compact && day_keys[i] == target_day)
                    .map(|i| &activities[i])
                    .collect();
                let rollup_line = format_rollup_line(
                    &members,
                    local_offset_ms,
                    day_narratives.get(&target_day).copied(),
                );
                // The rollup line replaces all that day's compact lines.
                let removed: usize = (0..n)
                    .filter(|&i| tiers[i] == Tier::Compact && day_keys[i] == target_day)
                    .map(|i| line_cost(&compact_lines[i]))
                    .sum();
                total = total - removed + line_cost(&rollup_line);
                for i in 0..n {
                    if tiers[i] == Tier::Compact && day_keys[i] == target_day {
                        tiers[i] = Tier::Rollup;
                    }
                }
            }
        }
    }

    // Materialize the lines in chronological order, collapsing each rollup DAY
    // to a single line at the position of its first (oldest) member.
    let mut body_lines: Vec<String> = Vec::new();
    let mut emitted_rollup_days: Vec<i64> = Vec::new();
    for i in 0..n {
        match tiers[i] {
            Tier::Full => body_lines.push(full_lines[i].clone()),
            Tier::Compact => body_lines.push(compact_lines[i].clone()),
            Tier::Rollup => {
                let day = day_keys[i];
                if emitted_rollup_days.contains(&day) {
                    continue;
                }
                emitted_rollup_days.push(day);
                let members: Vec<&Activity> = (0..n)
                    .filter(|&j| tiers[j] == Tier::Rollup && day_keys[j] == day)
                    .map(|j| &activities[j])
                    .collect();
                body_lines.push(format_rollup_line(
                    &members,
                    local_offset_ms,
                    day_narratives.get(&day).copied(),
                ));
            }
        }
    }

    // Step 4 (defensive, effectively unreachable): if even the fully-collapsed
    // body still exceeds the cap, drop the oldest body lines as a last resort
    // (the old behaviour) — bounded, never loops forever.
    let mut body_total: usize = body_lines.iter().map(|s| line_cost(s)).sum();
    while body_total > DIGEST_PROMPT_CHAR_CAP && body_lines.len() > 1 {
        body_total -= line_cost(&body_lines.remove(0));
        degraded = true;
    }

    let mut prompt = String::new();
    prompt.push_str(&format!(
        "Below are the user's Activity episodes for one {range_kind}, in chronological order \
(oldest first). Each line carries the local day, the local start–end clock time, the episode \
duration, optional category and focus labels, the title, and a short summary. Write the short \
(at most 2 sentences) second-person narrative digest, the headline, and the optional turn described in the \
instructions and return DigestNarrative.\n\n"
    ));
    prompt.push_str(&format!(
        "Range: {range_kind} [{range_start_ms} .. {range_end_ms}) ms ({} Activities)\n",
        activities.len()
    ));
    // Anchor the range in the user's LOCAL calendar (the per-line day labels are
    // already local) and state today, so the narrative speaks in the user's wall
    // time rather than the raw UTC millis above. `range_end_ms` is exclusive, so
    // the last covered day is `range_end_ms - 1`.
    prompt.push_str(&format!(
        "Local calendar: {} .. {} (the user's local time); today is {}. All day labels below are \
the user's local days.\n",
        day_label(range_start_ms, local_offset_ms),
        day_label(range_end_ms.saturating_sub(1), local_offset_ms),
        day_label(now_ms, local_offset_ms),
    ));
    prompt.push_str(&category_totals_line(activities));
    if degraded {
        prompt.push_str(
            "(Older activity is shown at reduced detail: the most recent episodes appear in full, \
earlier ones as brief one-line entries, and the oldest are summarized by local day. Nothing is \
missing — the Activity count above is the true total.)\n",
        );
    }
    prompt.push('\n');
    for line in &body_lines {
        prompt.push_str(line);
    }
    prompt
}

/// `Time by category: entertainment 41m (62%), research 18m (27%), …` over the
/// summed episode durations (overlapping episodes count twice; fine for a
/// shape). Without it the engine weighs lines equally, and a 10-second build in
/// an hour of videos read as "you focused on building".
pub(super) fn category_totals_line(activities: &[Activity]) -> String {
    let mut totals: Vec<(&str, i64)> = Vec::new();
    for a in activities {
        let label = a.category.map_or("uncategorized", category_label);
        let span = a.ended_at_ms.saturating_sub(a.started_at_ms).max(0);
        match totals.iter_mut().find(|(l, _)| *l == label) {
            Some((_, ms)) => *ms += span,
            None => totals.push((label, span)),
        }
    }
    let sum: i64 = totals.iter().map(|(_, ms)| ms).sum();
    if sum == 0 {
        return String::new();
    }
    totals.sort_by(|a, b| b.1.cmp(&a.1));
    let parts: Vec<String> = totals
        .iter()
        .map(|(label, ms)| format!("{label} {} ({}%)", duration_label(0, *ms), ms * 100 / sum))
        .collect();
    format!("Time by category: {}\n", parts.join(", "))
}

/// Char cap on the stored narrative: the prompt asks for ≤ 40 words (~240
/// chars, 2–3 lines in the lede), so this only catches an engine that ignores it.
pub(super) const NARRATIVE_CHAR_CAP: usize = 320;

/// Trim + validate an engine-returned narrative. `None` = generation failure
/// (the engine returned an empty/whitespace narrative). An over-long one keeps
/// its whole sentences that fit under [`NARRATIVE_CHAR_CAP`]; with no sentence
/// end inside the cap it falls back to a word cut ending in "…".
pub(super) fn normalize_narrative(raw: &str) -> Option<String> {
    let narrative = raw.trim();
    if narrative.is_empty() {
        return None;
    }
    if narrative.chars().count() <= NARRATIVE_CHAR_CAP {
        return Some(narrative.to_string());
    }
    // A sentence end is '.', '!' or '?' + whitespace, not before a lowercase word
    // and not with nothing before it, so "v0.1", "3.5h", "e.g. the" and a bare
    // "..." never count. Its whitespace may be the char just past the cap.
    let chars: Vec<(usize, char)> = narrative.char_indices().collect();
    let sentence_end = (0..NARRATIVE_CHAR_CAP)
        .map(|i| (chars[i], chars[i + 1].1))
        .filter(|&((_, c), next)| matches!(c, '.' | '!' | '?') && next.is_whitespace())
        .map(|((at, _), _)| at)
        .filter(|&at| {
            !narrative[at + 1..]
                .trim_start()
                .starts_with(char::is_lowercase)
        })
        .filter(|&at| narrative[..at].chars().any(char::is_alphanumeric))
        .last();
    Some(match sentence_end {
        Some(at) => narrative[..=at].to_string(),
        None => word_cut(narrative, NARRATIVE_CHAR_CAP),
    })
}

/// Cut `text` to at most `cap` chars at the last word boundary, ending in "…"
/// (a hard char cut when one word fills the cap).
pub(super) fn word_cut(text: &str, cap: usize) -> String {
    let capped: String = text.chars().take(cap - 1).collect();
    let cut = match capped.rfind(char::is_whitespace) {
        Some(at) if at > 0 => capped[..at].trim_end(),
        _ => capped.as_str(),
    };
    format!("{cut}…")
}

/// Char cap on the stored `headline — turn` string. An engine answer running
/// past this is truncated rather than rejected.
pub(super) const HEADLINE_CHAR_CAP: usize = 140;

/// The separator the stored headline carries between headline and turn; the
/// lede greys everything after its FIRST occurrence.
pub(super) const TURN_SEPARATOR: &str = " — ";

/// Join the engine's headline and optional turn into the one stored string
/// (`headline — turn`), clamped by [`normalize_headline`]. A dash the model put
/// in the headline anyway becomes a comma so the first [`TURN_SEPARATOR`] is
/// always the turn; a turn without a headline is dropped, and so is a turn that
/// would overrun the cap — a whole headline reads better than a clause cut short.
pub(super) fn joined_headline(headline: &str, turn: &str) -> Option<String> {
    let headline = headline.trim().replace(TURN_SEPARATOR, ", ");
    let turn = turn.trim().trim_start_matches(['—', '–', '-']).trim_start();
    if turn.is_empty() || headline.is_empty() {
        return normalize_headline(&headline);
    }
    let headline = headline.trim_end_matches('.');
    let joined = format!("{headline}{TURN_SEPARATOR}{turn}");
    if joined.chars().count() > HEADLINE_CHAR_CAP {
        return normalize_headline(headline);
    }
    normalize_headline(&joined)
}

/// Trim + validate an engine-returned headline. Unlike [`normalize_narrative`],
/// `None` is NOT a failure — a narrative-only digest stays valid. An over-long
/// headline is cut at the last word boundary within [`HEADLINE_CHAR_CAP`]
/// chars (a hard char cut when one headline-sized word fills the cap) and ends
/// in "…" so the cut reads as one, not as a sentence that stopped;
/// `truncate_chars` is not reused because its mid-word ellipsis reads wrong in
/// the large display type the headline is set in.
pub(super) fn normalize_headline(raw: &str) -> Option<String> {
    let headline = raw.trim();
    if headline.is_empty() {
        return None;
    }
    if headline.chars().count() <= HEADLINE_CHAR_CAP {
        return Some(headline.to_string());
    }
    Some(word_cut(headline, HEADLINE_CHAR_CAP))
}
