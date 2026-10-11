//! `hours:H-H` — the local time-of-day operator (Quick Recall's "afternoon").

use chrono::{Local, TimeZone};

use super::query::parse_search_query;
use super::test_support::*;
use super::{SearchCaptureRequest, SearchHoursRefinement};
use crate::AppInfra;

fn window(start_hour: u8, end_hour: u8) -> Option<SearchHoursRefinement> {
    Some(SearchHoursRefinement {
        start_hour,
        end_hour,
    })
}

#[test]
fn hours_operator_parses_plain_and_wrapping_windows_and_rejects_bad_ones() {
    assert_eq!(
        parse_search_query("pricing hours:12-18").hours,
        window(12, 18)
    );
    assert_eq!(parse_search_query("hours:22-6").hours, window(22, 6));
    let parsed = parse_search_query("pricing hours:12-18");
    assert_eq!(parsed.residual_query, "pricing");
    for bad in ["hours:12", "hours:9-9", "hours:12-25", "hours:noon-6"] {
        let parsed = parse_search_query(bad);
        assert!(parsed.hours.is_none(), "{bad}");
        assert_eq!(parsed.errors[0].kind, "bad_hours", "{bad}");
    }
}

/// A local wall-clock time today as RFC3339 (UTC), the way capture stores it.
fn local_today_at(hour: u32) -> String {
    let today = Local::now().date_naive();
    Local
        .from_local_datetime(&today.and_hms_opt(hour, 30, 0).unwrap())
        .earliest()
        .unwrap()
        .to_utc()
        .to_rfc3339()
}

#[test]
fn hours_operator_filters_hits_by_local_hour_and_echoes_the_refinement() {
    run_async_test(async {
        let dir = test_dir("hours-filter");
        let infra = AppInfra::initialize(&dir).await.expect("infra");
        let morning = seed_frame_with_text(
            &infra,
            "/tmp/hours-morning.jpg",
            &local_today_at(9),
            None,
            "budget review",
        )
        .await;
        let afternoon = seed_frame_with_text(
            &infra,
            "/tmp/hours-afternoon.jpg",
            &local_today_at(14),
            None,
            "budget review",
        )
        .await;
        let late = seed_frame_with_text(
            &infra,
            "/tmp/hours-late.jpg",
            &local_today_at(23),
            None,
            "budget review",
        )
        .await;

        let search = |query: &str| {
            let infra = &infra;
            let query = query.to_string();
            async move {
                infra
                    .search_capture(SearchCaptureRequest {
                        query,
                        frame_limit: Some(10),
                        frame_offset: None,
                        audio_limit: Some(0),
                        audio_offset: None,
                        snapshot_document_id: None,
                        refinements: None,
                        query_embedding: None,
                    })
                    .await
                    .expect("search")
            }
        };
        let ids = |response: &super::SearchCaptureResponse| {
            let mut ids: Vec<i64> = response
                .frames
                .iter()
                .map(|hit| hit.representative_frame.id)
                .collect();
            ids.sort();
            ids
        };

        let afternoon_only = search("budget hours:12-18").await;
        assert_eq!(ids(&afternoon_only), vec![afternoon.id]);
        assert_eq!(afternoon_only.applied_refinements.hours, window(12, 18));
        assert!(afternoon_only.parse_errors.is_empty());

        let night = search("budget hours:22-10").await;
        assert_eq!(ids(&night), vec![morning.id, late.id]);
    });
}
