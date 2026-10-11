//! Result counts per pill option: "Zed 4 · Slack 2 · Any app 9", "Drop 'Arc' ·
//! 1 moment". The same search with one refinement swapped out, its grouped
//! results counted by app / source / local day / part of day. Counts only —
//! the snippets the search builds are dropped, nothing else is fetched.

use chrono::{DateTime, Local, Timelike};
use serde::{Deserialize, Serialize};

use super::{SearchCaptureRefinements, SearchCaptureRequest, SearchStore};
use crate::Result;

/// Results counted per kind. ponytail: counts cap here (`capped`), the same page
/// cap the search itself has; a SQL COUNT path is the upgrade if "50+" shows up.
const FACET_RESULT_LIMIT: u32 = 50;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchFacet {
    /// Drops `app:`; buckets frames by app (audio has no app).
    App,
    /// Drops `source:`; buckets into `screen` / `microphone` / `system_audio`.
    Source,
    /// Drops the date range; buckets by local day, `YYYY-MM-DD`.
    Day,
    /// Drops `hours:`; buckets by local part of day: `morning` 5–12,
    /// `afternoon` 12–18, `evening` 18–24, `night` 0–5.
    PartOfDay,
}

impl SearchFacet {
    pub(super) fn clear(self, refinements: &mut SearchCaptureRefinements) {
        match self {
            Self::App => refinements.apps.clear(),
            Self::Source => {
                refinements.audio_sources.clear();
                refinements.screen_source = false;
            }
            Self::Day => refinements.date_range = None,
            Self::PartOfDay => refinements.hours = None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchFacetBucket {
    /// What to put back in the query: the app's bundle id (or name), the
    /// source kind, the ISO day, or the part of day.
    pub key: String,
    /// The app's display name; otherwise the same as `key`.
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchFacetCounts {
    pub facet: SearchFacet,
    /// Every result with this facet's refinement removed ("Any app 9").
    pub total: u32,
    /// A kind hit [`FACET_RESULT_LIMIT`]: counts are lower bounds.
    pub capped: bool,
    /// Most results first.
    pub buckets: Vec<SearchFacetBucket>,
}

impl SearchStore {
    /// Count what `request` would return with `facet`'s refinement swapped out
    /// (from the caller's refinements AND the query's operators), per bucket.
    pub async fn count_search_facet(
        &self,
        request: SearchCaptureRequest,
        facet: SearchFacet,
    ) -> Result<SearchFacetCounts> {
        let response = self
            .search_capture_without(
                SearchCaptureRequest {
                    frame_limit: Some(FACET_RESULT_LIMIT),
                    frame_offset: None,
                    audio_limit: Some(FACET_RESULT_LIMIT),
                    audio_offset: None,
                    ..request
                },
                Some(facet),
            )
            .await?;

        let frames = response.frames.iter().map(|frame| {
            let app = frame
                .app_bundle_id
                .clone()
                .or_else(|| frame.app_name.clone())
                .map(|key| (key.clone(), frame.app_name.clone().unwrap_or(key)));
            (
                app,
                "screen",
                frame.representative_frame.captured_at.as_str(),
            )
        });
        let audio = response.audio.iter().map(|audio| {
            let source = audio.source_kind.as_str();
            (None, source, audio.absolute_start_at.as_str())
        });

        let mut buckets: Vec<SearchFacetBucket> = Vec::new();
        let mut total = 0_u32;
        for (app, source, at) in frames.chain(audio) {
            total += 1;
            let local = DateTime::parse_from_rfc3339(at)
                .ok()
                .map(|instant| instant.with_timezone(&Local));
            let bucket = match facet {
                SearchFacet::App => app,
                SearchFacet::Source => Some((source.to_string(), source.to_string())),
                SearchFacet::Day => local.map(|at| {
                    let day = at.date_naive().to_string();
                    (day.clone(), day)
                }),
                SearchFacet::PartOfDay => local.map(|at| {
                    let part = part_of_day(at.hour()).to_string();
                    (part.clone(), part)
                }),
            };
            let Some((key, label)) = bucket else { continue };
            match buckets.iter_mut().find(|bucket| bucket.key == key) {
                Some(bucket) => bucket.count += 1,
                None => buckets.push(SearchFacetBucket {
                    key,
                    label,
                    count: 1,
                }),
            }
        }
        buckets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));

        Ok(SearchFacetCounts {
            facet,
            total,
            capped: response.has_more_frames || response.has_more_audio,
            buckets,
        })
    }
}

fn part_of_day(hour: u32) -> &'static str {
    match hour {
        5..=11 => "morning",
        12..=17 => "afternoon",
        18..=23 => "evening",
        _ => "night",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_support::*;
    use crate::AppInfra;
    use chrono::TimeZone;

    fn local_at(day: u32, hour: u32) -> String {
        Local
            .with_ymd_and_hms(2026, 5, day, hour, 30, 0)
            .earliest()
            .unwrap()
            .to_utc()
            .to_rfc3339()
    }

    fn request(query: &str) -> SearchCaptureRequest {
        SearchCaptureRequest {
            query: query.to_string(),
            frame_limit: None,
            frame_offset: None,
            audio_limit: None,
            audio_offset: None,
            snapshot_document_id: None,
            refinements: None,
            query_embedding: None,
        }
    }

    fn counts(facets: &SearchFacetCounts) -> Vec<(&str, u32)> {
        facets
            .buckets
            .iter()
            .map(|bucket| (bucket.key.as_str(), bucket.count))
            .collect()
    }

    #[test]
    fn facet_counts_swap_out_one_refinement_and_bucket_the_rest() {
        run_async_test(async {
            let dir = test_dir("search-facets");
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            let zed = Some(frame_with_app(Some("dev.zed.Zed"), Some("Zed")));
            let slack = Some(frame_with_app(
                Some("com.tinyspeck.slackmacgap"),
                Some("Slack"),
            ));
            seed_frame_with_text(
                &infra,
                "/tmp/f1.jpg",
                &local_at(5, 9),
                zed.clone(),
                "pricing",
            )
            .await;
            seed_frame_with_text(&infra, "/tmp/f2.jpg", &local_at(5, 14), zed, "pricing").await;
            seed_frame_with_text(&infra, "/tmp/f3.jpg", &local_at(6, 14), slack, "pricing").await;
            seed_frame_with_text(&infra, "/tmp/f4.jpg", &local_at(6, 15), None, "other").await;

            // The query's own `app:` operator is swapped out, not just the caller's.
            let apps = infra
                .search()
                .count_search_facet(request("pricing app:Slack"), SearchFacet::App)
                .await
                .unwrap();
            assert_eq!(apps.total, 3);
            assert!(!apps.capped);
            assert_eq!(
                counts(&apps),
                vec![("dev.zed.Zed", 2), ("com.tinyspeck.slackmacgap", 1)]
            );
            assert_eq!(apps.buckets[0].label, "Zed");

            let days = infra
                .search()
                .count_search_facet(request("pricing date:2026-05-05"), SearchFacet::Day)
                .await
                .unwrap();
            assert_eq!(counts(&days), vec![("2026-05-05", 2), ("2026-05-06", 1)]);

            // Other refinements stay: within Zed, by part of day, hours dropped.
            let parts = infra
                .search()
                .count_search_facet(
                    request("pricing app:Zed hours:12-18"),
                    SearchFacet::PartOfDay,
                )
                .await
                .unwrap();
            assert_eq!(counts(&parts), vec![("afternoon", 1), ("morning", 1)]);

            let sources = infra
                .search()
                .count_search_facet(request("pricing source:mic"), SearchFacet::Source)
                .await
                .unwrap();
            assert_eq!(counts(&sources), vec![("screen", 3)]);
            assert_eq!(serde_json::to_value(&sources).unwrap()["facet"], "source");
            assert_eq!(
                serde_json::to_value(SearchFacet::PartOfDay).unwrap(),
                "partOfDay"
            );
        });
    }
}
