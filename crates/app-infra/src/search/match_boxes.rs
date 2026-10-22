//! Where a query's words sit on one captured frame, read from the frame's stored
//! OCR geometry (`processing_results.structured_payload_json`). Quick Recall
//! zooms its crop onto these boxes and outlines them in Quick Look.

use ocr::OcrStructuredPayload;
use serde::{Deserialize, Serialize};

use super::{query::parse_search_query, SearchStore};
use crate::{processing::structured_payload_json_from_row, Result, OCR_PROCESSOR};

/// Boxes returned per frame — enough to aim a crop, few enough to outline.
const MAX_MATCH_BOXES: usize = 6;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchBoxKind {
    /// The observation contains a query word (drawn solid).
    Keyword,
    /// A meaning-only hit: the observation shares a word stem with the query
    /// (drawn dashed). There is no per-chunk provenance to do better — an anchor
    /// carries ONE pooled vector, not one per OCR line.
    Meaning,
}

/// One matched OCR observation, normalised to the frame (0–1) with a TOP-LEFT
/// origin (the stored geometry is Vision's lower-left; it is flipped here).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatchBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub kind: MatchBoxKind,
}

impl SearchStore {
    /// Match boxes for `query` on `frame_id`. `by_meaning` = the hit was
    /// meaning-only (`found_by_meaning`), so words are matched by stem. A frame
    /// with no OCR geometry yields no boxes, never an error.
    pub async fn frame_match_boxes(
        &self,
        frame_id: i64,
        query: &str,
        by_meaning: bool,
    ) -> Result<Vec<MatchBox>> {
        let terms = query_terms(query, by_meaning);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        // The frame's search anchor names the OCR result its text came from —
        // its own, or the equivalent frame's it reuses (same pixels, same boxes).
        let row = sqlx::query(
            "SELECT processing_results.processor, processing_results.structured_payload_json \
             FROM search_documents \
             JOIN processing_results ON processing_results.id = search_documents.processing_result_id \
             WHERE search_documents.anchor_type = 'frame' \
               AND search_documents.frame_id = ?1 \
               AND processing_results.processor = ?2 \
             ORDER BY search_documents.id DESC LIMIT 1",
        )
        .bind(frame_id)
        .bind(OCR_PROCESSOR)
        .fetch_optional(self.db.read())
        .await?;
        let Some(json) = row
            .as_ref()
            .map(structured_payload_json_from_row)
            .transpose()?
            .flatten()
        else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_str::<OcrStructuredPayload>(&json)
            .map(|payload| match_boxes(&payload, &terms, by_meaning))
            .unwrap_or_default())
    }
}

/// The words the query's FTS body matches (`ParsedQuery::match_terms`:
/// operators, `OR`, `-exclusions` and filler stopwords dropped). By meaning,
/// each word ≥ 4 chars is cut to a 4-char stem so "pricing" finds "Prices".
fn query_terms(raw: &str, by_meaning: bool) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in parse_search_query(raw).match_terms {
        let term = if by_meaning {
            if word.chars().count() < 4 {
                continue;
            }
            word.chars().take(4).collect()
        } else {
            word
        };
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

fn match_boxes(
    payload: &OcrStructuredPayload,
    terms: &[String],
    by_meaning: bool,
) -> Vec<MatchBox> {
    let flip = payload.coordinate_origin == "lower_left";
    let kind = if by_meaning {
        MatchBoxKind::Meaning
    } else {
        MatchBoxKind::Keyword
    };
    let mut scored: Vec<(usize, MatchBox)> = payload
        .observations
        .iter()
        .filter_map(|observation| {
            let text = observation.text.to_lowercase();
            let hits = terms
                .iter()
                .filter(|term| text.contains(term.as_str()))
                .count();
            let b = &observation.bounding_box;
            (hits > 0).then(|| {
                let y = if flip { 1.0 - b.y - b.height } else { b.y };
                (
                    hits,
                    MatchBox {
                        x: b.x,
                        y,
                        w: b.width,
                        h: b.height,
                        kind,
                    },
                )
            })
        })
        .collect();
    // Most query words first; the stable sort keeps reading order among ties.
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(MAX_MATCH_BOXES)
        .map(|(_, matched)| matched)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_support::*;
    use crate::{AppInfra, NewFrame, ProcessingJobDraft, ProcessingResultDraft};
    use ocr::{OcrBoundingBox, OcrObservation};

    #[test]
    fn query_terms_drop_operators_and_exclusions_and_stem_by_meaning() {
        assert_eq!(
            query_terms("\"Pricing page\" app:Arc hours:12-18 -draft x", false),
            vec!["pricing", "page"]
        );
        assert_eq!(query_terms("pricing page", true), vec!["pric", "page"]);
        // Filler words are not boxed — unless quoted or the whole query.
        assert_eq!(
            query_terms("kvm switch on desk", false),
            vec!["kvm", "switch", "desk"]
        );
        assert_eq!(query_terms("\"switch on\"", false), vec!["switch", "on"]);
        assert_eq!(query_terms("on the", false), vec!["on", "the"]);
    }

    #[test]
    fn frame_match_boxes_outline_matching_observations_top_left() {
        run_async_test(async {
            let dir = test_dir("match-boxes");
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/match-boxes.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame");
            let payload = OcrStructuredPayload::new(
                "apple_vision",
                None,
                vec![
                    OcrObservation::new(
                        "Pricing page",
                        0.9,
                        OcrBoundingBox::new(0.1, 0.7, 0.3, 0.1),
                    ),
                    OcrObservation::new("Unrelated", 0.9, OcrBoundingBox::new(0.5, 0.5, 0.2, 0.1)),
                    OcrObservation::new(
                        "Team pricing",
                        0.9,
                        OcrBoundingBox::new(0.1, 0.1, 0.2, 0.1),
                    ),
                ],
            );
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("job");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("Pricing page Unrelated Team pricing")
                    .with_structured_payload_json(serde_json::to_string(&payload).unwrap()),
            )
            .await;

            let boxes = infra
                .search()
                .frame_match_boxes(frame.id, "pricing page", false)
                .await
                .expect("boxes");
            assert_eq!(boxes.len(), 2, "the unrelated line is not outlined");
            // Both words first, then the one-word line; y flipped to top-left.
            assert!((boxes[0].y - 0.2).abs() < 1e-9, "{boxes:?}");
            assert!((boxes[1].y - 0.8).abs() < 1e-9, "{boxes:?}");
            assert_eq!(boxes[0].kind, MatchBoxKind::Keyword);

            let meaning = infra
                .search()
                .frame_match_boxes(frame.id, "prices", true)
                .await
                .expect("meaning boxes");
            assert_eq!(meaning.len(), 2);
            assert_eq!(meaning[0].kind, MatchBoxKind::Meaning);

            let none = infra
                .search()
                .frame_match_boxes(frame.id + 999, "pricing", false)
                .await
                .expect("missing frame");
            assert!(none.is_empty());
        });
    }
}
