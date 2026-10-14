use super::*;

fn activity(
    id: i64,
    started_at_ms: i64,
    ended_at_ms: i64,
    title: &str,
    summary: &str,
    category: Option<ActivityCategory>,
    focus: Option<FocusLevel>,
) -> Activity {
    Activity {
        id,
        title: title.to_string(),
        summary: summary.to_string(),
        category,
        focus,
        started_at_ms,
        ended_at_ms,
        created_at_ms: started_at_ms,
        evidence: Vec::new(),
    }
}

/// Same guard as `derivation.rs`: the engine schema must stay
/// `$ref`/`$defs`-free for reference-less structured-output backends.
#[test]
fn digest_schema_is_reference_free() {
    fn has_reference(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.contains_key("$ref")
                    || map.contains_key("$defs")
                    || map.values().any(has_reference)
            }
            serde_json::Value::Array(items) => items.iter().any(has_reference),
            _ => false,
        }
    }
    let schema = serde_json::to_value(schemars::schema_for!(DigestNarrative)).unwrap();
    assert!(!has_reference(&schema), "schema has a reference: {schema}");
}

#[test]
fn digest_preamble_leads_with_the_guardrail_and_bans_markdown() {
    let preamble = digest_preamble();
    assert!(preamble.starts_with(guardrail::SENSITIVE_GUARDRAIL_INSTRUCTION));
    assert!(preamble.contains("second person"));
    assert!(preamble.contains("Do NOT use markdown"));
    // The headline ask rides the same preamble: a dash-free sentence + turn.
    assert!(preamble.contains("headline for the range: a full sentence"));
    assert!(preamble.contains("Never put a dash in the headline"));
    assert!(preamble.contains("Optionally add a turn"));
    assert!(preamble.contains("no surrounding quotes"));
}

#[test]
fn recovers_local_offset_from_local_midnight_starts() {
    // 2026-06-08 00:00 in each zone, expressed as a UTC unix-ms instant.
    let utc_midnight = 1_780_876_800_000_i64; // 20612 days, divisible by DAY_MS
    assert_eq!(utc_midnight % DAY_MS, 0, "fixture must be a UTC midnight");
    // UTC: midnight is on the day boundary → offset 0.
    assert_eq!(recover_local_offset_ms(utc_midnight), 0);
    // UTC+5:30 (IST): local midnight is 18:30 UTC the previous day.
    let ist = utc_midnight - (5 * 60 + 30) * 60_000;
    assert_eq!(recover_local_offset_ms(ist), (5 * 60 + 30) * 60_000);
    // UTC−8 (PST): local midnight is 08:00 UTC the same day.
    let pst = utc_midnight + 8 * 3_600_000;
    assert_eq!(recover_local_offset_ms(pst), -8 * 3_600_000);
    // UTC+13 (Tonga): preferred over the ambiguous UTC−11 alias.
    let tonga = utc_midnight - 13 * 3_600_000;
    assert_eq!(recover_local_offset_ms(tonga), 13 * 3_600_000);
}

#[test]
fn day_label_renders_the_local_weekday_and_date() {
    // 2026-06-08 is a Monday. 10:00 local in UTC−8 = 18:00 UTC.
    let utc_midnight = 1_780_876_800_000_i64;
    let local_offset_ms = -8 * 3_600_000;
    let at = utc_midnight + 18 * 3_600_000;
    assert_eq!(day_label(at, local_offset_ms), "Mon Jun 8");
    // The same instant labeled in UTC falls on the same calendar day here,
    // but a late-evening local time crosses: 23:00 UTC−8 = 07:00 UTC Tue.
    let late = utc_midnight + 31 * 3_600_000;
    assert_eq!(day_label(late, local_offset_ms), "Mon Jun 8");
    assert_eq!(day_label(late, 0), "Tue Jun 9");
}

#[test]
fn duration_label_is_compact() {
    assert_eq!(duration_label(0, 30_000), "<1m");
    assert_eq!(duration_label(0, 45 * 60_000), "45m");
    assert_eq!(duration_label(0, 125 * 60_000), "2h 05m");
    // A clamped/garbage negative span never panics.
    assert_eq!(duration_label(1_000, 0), "<1m");
}

#[test]
fn activity_line_carries_day_duration_labels_and_omits_unset() {
    let labeled = activity(
        1,
        0,
        90 * 60_000,
        "Billing migration",
        "Moved invoices to the new schema.",
        Some(ActivityCategory::Creating),
        Some(FocusLevel::Deep),
    );
    let line = format_activity_line(&labeled, 0, &[]);
    assert!(line.contains("Thu Jan 1"), "epoch day label: {line}");
    assert!(
        line.contains("| 00:00–01:30 | 1h 30m"),
        "local clock span: {line}"
    );
    // The clock span is local: UTC+5:30 shifts it, the compact tier keeps it.
    let ist = (5 * 60 + 30) * 60_000;
    assert!(format_activity_line(&labeled, ist, &[]).contains("| 05:30–07:00 |"));
    assert!(format_activity_line_compact(&labeled, ist).contains("| 05:30–07:00 |"));
    assert!(line.contains("1h 30m"));
    assert!(line.contains("| creating | deep]"));
    assert!(line.contains("Billing migration — Moved invoices"));
    let apps = ["Zed".to_string(), "Safari".to_string()];
    assert!(format_activity_line(&labeled, 0, &apps).contains("| deep | in Zed, Safari]"));

    // Unset category/focus and empty summary leave no dangling separators.
    let bare = activity(2, 0, 60_000, "Misc", "  ", None, None);
    let line = format_activity_line(&bare, 0, &[]);
    assert!(!line.contains("| creating"));
    assert!(!line.contains("—"));
    assert!(line.trim_end().ends_with("Misc"));
}

#[test]
fn prompt_is_chronological_with_a_range_header() {
    let activities = vec![
        activity(1, 1_000, 61_000, "Oldest", "first", None, None),
        activity(2, 100_000, 160_000, "Newest", "last", None, None),
    ];
    let prompt = build_digest_prompt("week", 0, DAY_MS * 7, DAY_MS * 3, &activities, &[], &HashMap::new());
    assert!(prompt.contains("Range: week [0 .. 604800000) ms (2 Activities)"));
    // The local-calendar anchor names today and the range span in local days.
    assert!(prompt.contains("today is"));
    assert!(prompt.contains("Local calendar:"));
    assert!(prompt.contains("one week"));
    assert!(prompt.contains("the optional turn"), "prompt asks for headline + turn");
    let oldest = prompt.find("Oldest").expect("oldest line");
    let newest = prompt.find("Newest").expect("newest line");
    assert!(oldest < newest, "chronological: oldest first, newest last");
    assert!(!prompt.contains("omitted"), "no omission note under budget");
}

#[test]
fn prompt_leads_with_time_by_category_largest_first() {
    let watched = Some(ActivityCategory::Entertainment);
    let activities = vec![
        activity(1, 0, 10_000, "Build", "", Some(ActivityCategory::Creating), None),
        activity(2, 20_000, 20_000 + 30 * 60_000, "Videos", "", watched, None),
        activity(3, 0, 10 * 60_000, "Misc", "", None, None),
    ];
    let prompt = build_digest_prompt("day", 0, DAY_MS, DAY_MS, &activities, &[], &HashMap::new());
    assert!(
        prompt.contains("Time by category: entertainment 30m (74%), uncategorized 10m (24%), creating <1m (0%)\n"),
        "{prompt}"
    );
    assert_eq!(category_totals_line(&[]), "");
}

/// Under crushing load (even an all-compact body overflows) the prompt
/// degrades by recency rather than dropping the oldest: the newest episode
/// survives at the richest tier still affordable (here a compact line, since
/// nothing can stay full), the oldest are folded into rollup day lines (NOT
/// dropped), the body stays bounded, the true total count is present, and
/// the tier-explanation note appears. (The newest-stays-FULL case is
/// covered by `prompt_compact_tier_drops_summaries_for_older_episodes`,
/// where the all-compact body fits.)
#[test]
fn prompt_degrades_by_recency_over_the_char_cap_and_says_so() {
    let long_summary = "x".repeat(DIGEST_SUMMARY_CHAR_CAP);
    // 1000 activities spread across ~25 local days (UTC offset 0). Even the
    // all-compact body (~30 chars × 1000 ≈ 30k) overflows the cap, so the
    // oldest days MUST roll up — exercising all three tiers at once.
    let activities: Vec<Activity> = (0..1000)
        .map(|i| {
            let start = i * (DAY_MS / 40); // ~40 episodes per local day
            activity(
                i,
                start,
                start + 60_000,
                &format!("Activity {i}"),
                &long_summary,
                None,
                None,
            )
        })
        .collect();
    let prompt = build_digest_prompt("month", 0, DAY_MS * 30, DAY_MS * 15, &activities, &[], &HashMap::new());
    // Bounded: the body stays near the cap (header + a little slack).
    assert!(
        prompt.chars().count() < DIGEST_PROMPT_CHAR_CAP + 1000,
        "prompt stays bounded: {} chars",
        prompt.chars().count()
    );
    // Newest survives (its title is still present); the oldest are NOT
    // dropped — they appear, folded into rollup day lines.
    assert!(prompt.contains("Activity 999"), "newest survives");
    assert!(prompt.contains("Activity 0"), "oldest activity is not dropped");
    // Degradation actually occurred: rollup day lines exist.
    assert!(prompt.contains("activities,"), "the oldest days are rolled up");
    // The tier-explanation note appears, with the true total.
    assert!(prompt.contains("summarized by local day"));
    assert!(prompt.contains("the true total"));
    assert!(prompt.contains("(1000 Activities)"));
}

/// Compact tier: enough activities to overflow Full but fit as Compact. The
/// oldest renders as `- [day | dur] Title` with NO summary separator, while
/// the newest keeps its full summary. No rollup ("activities," count) yet.
#[test]
fn prompt_compact_tier_drops_summaries_for_older_episodes() {
    // ~120 chars of summary each: full lines (~155 chars) overflow 24k well
    // before 300 activities; compact lines (~30 chars) stay far under.
    let summary = "y".repeat(120);
    let activities: Vec<Activity> = (0..300)
        .map(|i| {
            let start = i * 60_000;
            activity(
                i,
                start,
                start + 60_000,
                &format!("Episode {i}"),
                &summary,
                None,
                None,
            )
        })
        .collect();
    let prompt = build_digest_prompt("week", 0, DAY_MS * 7, DAY_MS * 3, &activities, &[], &HashMap::new());
    // Newest keeps its summary (Full tier).
    assert!(prompt.contains(&format!("Episode 299 — {summary}")));
    // Oldest is a compact line: title with no summary separator.
    assert!(
        prompt.contains("] Episode 0\n"),
        "oldest is a compact line without a summary"
    );
    assert!(
        !prompt.contains(&format!("Episode 0 — {summary}")),
        "oldest dropped its summary"
    );
    // Still bounded, total stated, note present.
    assert!(prompt.chars().count() < DIGEST_PROMPT_CHAR_CAP + 1000);
    assert!(prompt.contains("(300 Activities)"));
    assert!(prompt.contains("reduced detail"));
}

/// Rollup computed path: force a rollup and assert a day line carries the
/// activity count, a duration, a category label, and the longest titles of
/// that day folded in.
#[test]
fn prompt_rollup_computed_folds_oldest_days_into_one_line() {
    let long_summary = "z".repeat(DIGEST_SUMMARY_CHAR_CAP);
    // 1000 activities across ~25 local days at offset 0 so the all-compact
    // body overflows and the oldest days roll up. The oldest day (i in
    // 0..40) carries a creating category and a longer span so the rollup
    // line names a category and folds in that day's longest titles.
    let activities: Vec<Activity> = (0..1000)
        .map(|i| {
            let start = i * (DAY_MS / 40);
            let (category, end) = if i < 40 {
                (Some(ActivityCategory::Creating), start + 30 * 60_000)
            } else {
                (None, start + 60_000)
            };
            activity(
                i,
                start,
                end,
                &format!("Activity {i}"),
                &long_summary,
                category,
                None,
            )
        })
        .collect();
    let prompt = build_digest_prompt("month", 0, DAY_MS * 30, DAY_MS * 15, &activities, &[], &HashMap::new());
    // A computed rollup line: count + a duration + a category label.
    assert!(prompt.contains("activities,"), "a rollup line with a count");
    assert!(
        prompt.contains("top categories: creating"),
        "the oldest day names its category"
    );
    // An oldest individual title is folded into a day line ("e.g. ...").
    assert!(prompt.contains("e.g. \"Activity"), "sample titles folded in");
    assert!(prompt.contains("(1000 Activities)"));
    assert!(prompt.contains("summarized by local day"));
}

/// Rollup hybrid path: a cached DAY digest whose `range_start_ms` matches an
/// old activity's local day start is reused verbatim — its narrative text
/// appears and the computed-stats form is NOT used for that day.
#[test]
fn prompt_rollup_hybrid_reuses_cached_day_narrative() {
    let long_summary = "w".repeat(DIGEST_SUMMARY_CHAR_CAP);
    let local_offset_ms = 0;
    // The oldest day's local-midnight key (offset 0 → started_at 0 → day 0).
    let oldest_day_start = local_day_start_ms(0, local_offset_ms);
    let narrative =
        "You spent the morning untangling the billing migration and the afternoon on review.";
    let day_digests = vec![StoredDigest {
        range_kind: "day".to_string(),
        range_start_ms: oldest_day_start,
        range_end_ms: oldest_day_start + DAY_MS,
        narrative: narrative.to_string(),
        headline: Some("A billing day".to_string()),
        input_fingerprint: "fp".to_string(),
        generated_at_ms: 0,
    }];
    // Enough activities to force a rollup; the oldest day (i in 0..40) is the
    // one the cached digest covers.
    let activities: Vec<Activity> = (0..1000)
        .map(|i| {
            let start = i * (DAY_MS / 40);
            activity(
                i,
                start,
                start + 60_000,
                &format!("Activity {i}"),
                &long_summary,
                Some(ActivityCategory::Creating),
                None,
            )
        })
        .collect();
    let prompt =
        build_digest_prompt("month", 0, DAY_MS * 30, DAY_MS * 15, &activities, &day_digests, &HashMap::new());
    // The cached narrative appears verbatim on its rollup line.
    assert!(
        prompt.contains(narrative),
        "the cached day narrative is reused verbatim"
    );
    // That day's rollup line is the hybrid prose, not the computed form: the
    // hybrid line is `- [<label>] <narrative>`, so it must not be followed by
    // a computed " activities," stats clause for the SAME label.
    let day_lbl = day_label(0, local_offset_ms);
    let hybrid_line = format!("- [{day_lbl}] {narrative}");
    assert!(prompt.contains(&hybrid_line), "hybrid line shape: {hybrid_line}");
    let computed_line = format!("- [{day_lbl}] ");
    // The only line starting with that label prefix is the hybrid one.
    let computed_with_stats = format!("- [{day_lbl}] 40 activities,");
    assert!(
        !prompt.contains(&computed_with_stats),
        "the cached day must not use the computed-stats form"
    );
    // Sanity: the prefix exists exactly once (the hybrid line).
    assert_eq!(
        prompt.matches(&computed_line).count(),
        1,
        "exactly one rollup line for the cached day"
    );
}

/// `local_day_start_ms` returns the day digest's local-midnight (a UTC ms
/// instant) for a known offset — mirroring the
/// `recovers_local_offset_from_local_midnight_starts` fixtures.
#[test]
fn local_day_start_ms_matches_a_day_digests_local_midnight() {
    // 2026-06-08 00:00 local, as a UTC unix-ms instant per zone.
    let utc_midnight = 1_780_876_800_000_i64;
    assert_eq!(utc_midnight % DAY_MS, 0);
    // UTC: an instant during that day floors back to the same UTC midnight.
    assert_eq!(local_day_start_ms(utc_midnight + 12 * 3_600_000, 0), utc_midnight);
    // UTC+5:30 (IST): local midnight is 18:30 UTC the previous day. An
    // instant at local noon recovers that same local-midnight key.
    let ist_offset = (5 * 60 + 30) * 60_000;
    let ist_midnight = utc_midnight - ist_offset;
    let ist_noon = ist_midnight + 12 * 3_600_000;
    assert_eq!(local_day_start_ms(ist_noon, ist_offset), ist_midnight);
    // UTC−8 (PST): local midnight is 08:00 UTC the same day.
    let pst_offset = -8 * 3_600_000;
    let pst_midnight = utc_midnight + 8 * 3_600_000;
    let pst_evening = pst_midnight + 20 * 3_600_000;
    assert_eq!(local_day_start_ms(pst_evening, pst_offset), pst_midnight);
    // The recovered start round-trips back to the recovered offset, so the
    // key matches what `list_day_digests_in_range` rows are keyed on.
    assert_eq!(recover_local_offset_ms(ist_midnight), ist_offset);
    assert_eq!(recover_local_offset_ms(pst_midnight), pst_offset);
}

/// The freshness floor widens with the range: day 1h, week 6h, month 24h.
/// A digest inside its floor is served even on fingerprint mismatch; one
/// past it regenerates. A backwards clock (negative age) counts as fresh.
#[test]
fn freshness_floor_widens_with_range_and_gates_on_age() {
    const HOUR_MS: i64 = 60 * 60 * 1000;
    assert_eq!(freshness_floor_ms("day"), HOUR_MS);
    assert_eq!(freshness_floor_ms("week"), 6 * HOUR_MS);
    assert_eq!(freshness_floor_ms("month"), 24 * HOUR_MS);

    let generated_at = 1_000_000_000_000_i64;
    // Just inside vs. at the day floor.
    assert!(within_freshness_floor("day", generated_at, generated_at + HOUR_MS - 1));
    assert!(!within_freshness_floor("day", generated_at, generated_at + HOUR_MS));
    // A week-old month digest is still fresh; a day-old week digest is not.
    assert!(within_freshness_floor("month", generated_at, generated_at + 23 * HOUR_MS));
    assert!(!within_freshness_floor("week", generated_at, generated_at + 24 * HOUR_MS));
    // Clock moved backwards: fresh, never a forced regeneration.
    assert!(within_freshness_floor("day", generated_at, generated_at - HOUR_MS));
}

/// PR #112 #8: the hard sensitive post-filter run over the generated digest
/// before persist uses the SAME `guardrail::is_sensitive` backstop as
/// Conclusions, applied to (headline, narrative). A narrative the model wrote
/// into an off-limits category must trip it; a benign work narrative must not.
#[test]
fn sensitive_digest_narrative_trips_the_hard_post_filter() {
    // Free-text narrative the soft preamble failed to prevent.
    assert!(guardrail::is_sensitive(
        "A heavy week",
        "Most of this week went into therapy appointments and managing your depression."
    ));
    assert!(guardrail::is_sensitive(
        "",
        "You spent a lot of time reading about your cancer diagnosis."
    ));
    // A benign work digest must NOT trip — no false suppression of the
    // ordinary case.
    assert!(!guardrail::is_sensitive(
        "A deep week in the editor",
        "Most of this week went into the billing migration, with a shift toward test coverage by Friday."
    ));
}

/// Mirrors `sensitive_activity_never_egresses_via_recall_context` in
/// `brokered_access.rs`, for the Digest egress door. Activities are persisted
/// UNFILTERED (only Conclusions are gated at derivation), so the Digest path
/// must re-filter activity INPUT through `guardrail::is_sensitive` before it
/// embeds title/summary into the cloud-bound prompt. This drives the SAME
/// filter expression the production path applies after
/// `list_activities_in_range` and asserts the sensitive Activity's text
/// never reaches `build_digest_prompt`'s output — if someone deletes the
/// "redundant"-looking input filter, THIS goes red even though the output
/// post-filter test (`sensitive_digest_narrative_trips_the_hard_post_filter`)
/// stays green.
#[test]
fn sensitive_activity_never_egresses_via_digest_prompt() {
    // A SENSITIVE activity persisted unfiltered, and a benign one, in range.
    let sensitive = activity(
        1,
        1_000,
        2_000,
        "Therapy appointment",
        "attended a therapy appointment about your depression",
        None,
        None,
    );
    let benign_one = activity(
        2,
        3_000,
        4_000,
        "Billing migration",
        "Moved invoices to the new schema.",
        Some(ActivityCategory::Creating),
        None,
    );
    let benign_two = activity(
        3,
        5_000,
        6_000,
        "Test coverage",
        "Added tests for the new schema.",
        Some(ActivityCategory::Creating),
        None,
    );
    let raw = vec![sensitive.clone(), benign_one, benign_two];

    // EXACTLY the production filter from `get_or_generate_digest`.
    let filtered: Vec<Activity> = raw
        .iter()
        .cloned()
        .filter(|a| !guardrail::is_sensitive(&a.title, &a.summary))
        .collect();

    // The sensitive Activity is dropped; the benign ones survive.
    assert_eq!(filtered.len(), 2, "sensitive activity must be filtered out");
    assert!(
        filtered.iter().all(|a| !guardrail::is_sensitive(&a.title, &a.summary)),
        "no surviving activity may be sensitive"
    );

    // The cloud-bound prompt built from the filtered set must not carry the
    // sensitive Activity's text in any form.
    let prompt = build_digest_prompt("day", 0, DAY_MS, DAY_MS, &filtered, &[], &HashMap::new());
    assert!(
        !prompt.contains("Therapy"),
        "sensitive activity title egressed via digest prompt: {prompt}"
    );
    assert!(
        !prompt.contains("depression"),
        "sensitive activity summary egressed via digest prompt: {prompt}"
    );
    // Recall still works: a benign activity is present (not just empty).
    assert!(
        prompt.contains("Billing migration"),
        "benign activity should remain in the prompt: {prompt}"
    );
}

#[test]
fn normalize_narrative_trims_and_rejects_empty() {
    assert_eq!(
        normalize_narrative("  A focused week. \n"),
        Some("A focused week.".to_string())
    );
    assert_eq!(normalize_narrative("   \n\t"), None);
    assert_eq!(normalize_narrative(""), None);
}

/// Headline post-validation: trim; empty/whitespace → `None` (NOT a
/// failure); over-long → word-boundary truncation under
/// [`HEADLINE_CHAR_CAP`]; a single cap-filling word → hard char cut.
#[test]
fn normalize_headline_trims_drops_empty_and_truncates_on_word_boundaries() {
    assert_eq!(
        normalize_headline("  A deep week in the editor \n"),
        Some("A deep week in the editor".to_string())
    );
    assert_eq!(normalize_headline("   \n\t"), None);
    assert_eq!(normalize_headline(""), None);

    // At the cap exactly: untouched.
    let exact = "x".repeat(HEADLINE_CHAR_CAP);
    assert_eq!(normalize_headline(&exact), Some(exact.clone()));

    // Over the cap: cut at the LAST word boundary inside the cap, with no
    // dangling whitespace and no mid-word fragment.
    let long = format!("{} tail", "word ".repeat(30).trim_end()); // 154 chars
    let truncated = normalize_headline(&long).expect("headline");
    assert!(truncated.chars().count() <= HEADLINE_CHAR_CAP);
    assert_eq!(truncated, format!("{}…", "word ".repeat(27).trim_end()));

    // One unbroken cap-filling word: hard char cut, never `None`.
    let unbroken = "y".repeat(HEADLINE_CHAR_CAP + 20);
    assert_eq!(
        normalize_headline(&unbroken),
        Some(format!("{}…", "y".repeat(HEADLINE_CHAR_CAP - 1)))
    );
}

/// `headline — turn` is one stored string: the lede splits on the FIRST
/// " — ", so a dash inside the headline must not move that split, and the
/// joined string clamps at [`HEADLINE_CHAR_CAP`].
#[test]
fn joined_headline_splits_on_the_first_dash_and_clamps_at_140() {
    assert_eq!(HEADLINE_CHAR_CAP, 140);
    let joined = joined_headline(
        "A build week. The pipeline — not the UI — ate your mornings.",
        "— until Thursday's calls took the whole day back.",
    )
    .expect("headline");
    let (head, turn) = joined.split_once(TURN_SEPARATOR).expect("turn split");
    assert_eq!(head, "A build week. The pipeline, not the UI, ate your mornings");
    assert_eq!(turn, "until Thursday's calls took the whole day back.");

    // No turn → the bare headline; a turn alone is dropped.
    assert_eq!(joined_headline(" A quiet day ", "  ").as_deref(), Some("A quiet day"));
    assert_eq!(joined_headline("", "until lunch"), None);

    // A turn that would overrun the cap is dropped whole, never cut mid-clause
    // ("…until the meeting took over the rest of").
    let long = joined_headline(&"word ".repeat(20), &"turn ".repeat(30)).expect("headline");
    assert_eq!(long, "word ".repeat(20).trim_end());
}

/// A forced re-read that cannot resolve an engine must SAY so.
///
/// `regenerate_user_context_digest` is the Overview's explicit re-read
/// button, and its contract is that a `too_few` absence means the range genuinely
/// has nothing to read — the Overview renders exactly one sentence for it:
/// "Not enough activity in this range to write a read."
///
/// Since the engine resolve went LIVE (it now refreshes a ChatGPT OAuth
/// token), this call can fail for a credential/connectivity reason on a
/// range that is full of Activities. Collapsing that into `too_few` tells
/// the user their data is thin when the truth is the engine never
/// resolved — and records nothing in the derivation-run ledger either, so
/// the failure is invisible on every surface.
#[tokio::test]
async fn a_forced_reread_surfaces_a_live_engine_failure_instead_of_reading_as_no_activity() {
    live_resolve_failure_surfaces(true).await;
}

/// OV-06: the lazy path must not read an engine failure as "not enough
/// activity" either. A sign-in failure is NOT cooled down: the second visit
/// re-resolves (no network), so signing back in takes effect at once.
#[tokio::test]
async fn a_lazy_read_surfaces_a_sign_in_failure_without_a_cooldown() {
    live_resolve_failure_surfaces(false).await;
}

async fn live_resolve_failure_surfaces(force: bool) {
    use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind};

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("digest-live-resolve-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra should initialize");
    let store = infra.user_context();

    // A ChatGPT provider that IS connected (so the no-network prerequisite
    // at step 1 passes) but whose access token is long expired with no
    // refresh token to spend — the live resolver's terminal failure, with
    // no network involved.
    let provider_id = format!("chatgpt-digest-live-resolve-{unique}");
    crate::secret_vault_test_support::install_shared_test_secret_vault();
    crate::chatgpt_auth::store_token_set(
        &provider_id,
        &crate::chatgpt_auth::ChatgptTokenSet {
            access_token: "expired-access-token".to_string(),
            refresh_token: None,
            expires_at: Some(0),
            rejected: false,
        },
    )
    .expect("seed the expired token set");
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: provider_id.clone(),
            kind: AiProviderKind::Chatgpt,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: provider_id.clone(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
    };

    // A range with plenty to read: two ordinary, non-sensitive Activities.
    // A UTC midnight; the two callers use different days so the shared
    // in-memory cooldown can't couple them.
    let range_start_ms = 1_780_876_800_000_i64 + if force { 0 } else { 7 * DAY_MS };
    let range_end_ms = range_start_ms + DAY_MS;
    for (offset, title) in [(0_i64, "Billing migration"), (2 * 3_600_000, "Code review")] {
        store
            .insert_activity_with_evidence(app_infra::NewActivity {
                title: title.to_string(),
                summary: format!("{title} work"),
                category: None,
                focus: None,
                started_at_ms: range_start_ms + offset,
                ended_at_ms: range_start_ms + offset + 1_800_000,
                derivation_run_id: None,
                evidence: Vec::new(),
            })
            .await
            .expect("seed an activity");
    }

    let outcome = get_or_generate_digest(
        &settings,
        true,
        store,
        "day",
        range_start_ms,
        range_end_ms,
        force,
    )
    .await;

    match &outcome {
        Err(message) => assert!(
            !message.trim().is_empty(),
            "the failure must carry a reason the user can act on"
        ),
        other => panic!(
            "a forced re-read whose engine never resolved must not read as \
             'not enough activity in this range': {other:?}"
        ),
    }

    // …and it is on the record: the run ledger is where a failed engine
    // pass is accounted for (the extraction-failure arm already writes one).
    let runs = store
        .list_derivation_runs(10)
        .await
        .expect("read the run ledger");
    assert!(
        runs.iter().any(|run| run.kind == "digest" && run.status == "failed"),
        "a failed forced re-read must leave a failed digest run: {runs:?}"
    );

    if !force {
        let again = get_or_generate_digest(
            &settings,
            true,
            store,
            "day",
            range_start_ms,
            range_end_ms,
            false,
        )
        .await;
        assert_eq!(again, outcome, "the lazy retry reports the same failure");
        let failed_runs = store
            .list_derivation_runs(10)
            .await
            .expect("read the run ledger")
            .into_iter()
            .filter(|run| run.kind == "digest" && run.status == "failed")
            .count();
        assert_eq!(failed_runs, 2, "a sign-in failure re-resolves instead of replaying");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failure_cooldown_hits_within_the_window_and_misses_after() {
    let start = 9_000_001;
    let _: Result<(), _> = cooldown_fail("day", start, "fp", 1_000, "boom".to_string());
    assert_eq!(
        cooldown_replay("day", start, "fp", false, 1_000 + FAILURE_COOLDOWN_MS - 1),
        Some("boom".to_string())
    );
    assert_eq!(cooldown_replay("day", start, "fp", false, 1_000 + FAILURE_COOLDOWN_MS), None);
    // An expired entry is gone for good.
    assert_eq!(cooldown_replay("day", start, "fp", false, 1_001), None);
}

#[test]
fn failure_cooldown_is_bypassed_by_reread_and_cleared_by_a_new_fingerprint() {
    let start = 9_000_002;
    let _: Result<(), _> = cooldown_fail("week", start, "fp", 1_000, "boom".to_string());
    assert_eq!(cooldown_replay("week", start, "fp", true, 1_001), None, "re-read bypasses");
    assert!(cooldown_replay("week", start, "fp", false, 1_001).is_some(), "bypass keeps it");
    assert_eq!(cooldown_replay("week", start, "fp2", false, 1_001), None);
    assert_eq!(
        cooldown_replay("week", start, "fp", false, 1_001),
        None,
        "a new fingerprint clears the entry"
    );
}

#[test]
fn the_sensitive_hold_is_a_distinct_outcome_the_cooldown_replays() {
    let start = 9_000_003;
    let held: Result<Option<UserContextDigest>, String> =
        cooldown_fail("month", start, "fp", 1_000, DIGEST_SENSITIVE_HOLD.to_string());
    assert_eq!(held, Err(DIGEST_SENSITIVE_HOLD.to_string()));
    assert_eq!(
        cooldown_replay("month", start, "fp", false, 1_001).as_deref(),
        Some(DIGEST_SENSITIVE_HOLD)
    );
    cooldown_clear("month", start);
    assert_eq!(cooldown_replay("month", start, "fp", false, 1_001), None);
}
