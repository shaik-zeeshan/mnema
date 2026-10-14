//! Moment-level clean-up of grouped Capture Search results. Runs inside
//! grouping — before pagination — so `has_more_*`, Quick Recall and the broker
//! all see one list:
//! - short audio fragments stop outranking clear screen matches,
//! - a mic moment that is the speakers' echo of a system-audio moment drops,
//! - screen moments from one app within the same minute fold into one.
//!
//! Every threshold lives here. Never logs: the inputs are captured text.

use std::collections::HashSet;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::{AudioSearchResult, FrameSearchResult};
use crate::AudioSegmentSourceKind;

/// An audio fragment under this many words is "short": BM25's length
/// normalisation scores a 3-word "EMS KVM switch," above every long page that
/// says the same words, so a short fragment's rank is scaled by words / this.
const AUDIO_SHORT_FRAGMENT_WORDS: usize = 6;
/// Every audio rank is scaled by this, so audio has to beat screen by a clear
/// margin (~25%) to take the best-moment slot; close scores go to the screen.
const AUDIO_VS_SCREEN_MARGIN: f64 = 0.8;
/// A mic and a system-audio moment this close in time (either side)...
const ECHO_TIME_SLACK_MS: i64 = 2_000;
/// ...whose normalised-token Jaccard similarity reaches this are one sound
/// heard twice — the mic picking up the speakers. The system copy is kept.
const ECHO_TEXT_SIMILARITY: f64 = 0.6;
/// Screen moments from the same app within this many seconds of a better one
/// fold into it.
const SCREEN_MOMENT_WINDOW_SECS: i64 = 60;

/// Weakens a lower-is-better rank by `factor` in (0, 1]. Ranks are negative
/// (BM25, negated RRF), so weakening scales toward zero.
fn weaken(rank: f64, factor: f64) -> f64 {
    if rank < 0.0 {
        rank * factor
    } else {
        rank / factor
    }
}

/// The lowercased words of a snippet, `<mark>` tags dropped.
fn snippet_words(snippet: &str) -> Vec<String> {
    snippet
        .replace("<mark>", " ")
        .replace("</mark>", " ")
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn parse_at(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn sort_audio_moments(results: &mut [AudioSearchResult]) {
    results.sort_by(|a, b| {
        a.rank
            .total_cmp(&b.rank)
            .then_with(|| b.absolute_start_at.cmp(&a.absolute_start_at))
            .then_with(|| a.group_key.cmp(&b.group_key))
    });
}

/// Short-fragment penalty + screen margin, then echo removal; best-first out.
pub(super) fn rank_audio_moments(mut results: Vec<AudioSearchResult>) -> Vec<AudioSearchResult> {
    for result in &mut results {
        // A truncated FTS/meaning snippet (`...`/`…`) means a longer fragment.
        let truncated = result.snippet.contains("...") || result.snippet.contains('…');
        let words = snippet_words(&result.snippet).len();
        let short = if !truncated && words < AUDIO_SHORT_FRAGMENT_WORDS {
            words.max(1) as f64 / AUDIO_SHORT_FRAGMENT_WORDS as f64
        } else {
            1.0
        };
        result.rank = weaken(result.rank, short * AUDIO_VS_SCREEN_MARGIN);
    }
    let mut kept = drop_mic_echoes(results);
    sort_audio_moments(&mut kept);
    kept
}

/// Drops each mic moment that echoes a system-audio moment, handing its rank to
/// the system copy when better (the moment keeps its place; the clean source
/// plays). ponytail: O(mic × system) scan; sort by time if pages get huge.
fn drop_mic_echoes(mut results: Vec<AudioSearchResult>) -> Vec<AudioSearchResult> {
    let spans: Vec<_> = results
        .iter()
        .map(|r| parse_at(&r.absolute_start_at).zip(parse_at(&r.absolute_end_at)))
        .collect();
    let words: Vec<HashSet<String>> = results
        .iter()
        .map(|r| snippet_words(&r.snippet).into_iter().collect())
        .collect();
    let slack = time::Duration::milliseconds(ECHO_TIME_SLACK_MS);
    let mut echoed = vec![false; results.len()];
    for mic in 0..results.len() {
        if results[mic].source_kind != AudioSegmentSourceKind::Microphone {
            continue;
        }
        let Some((mic_start, mic_end)) = spans[mic] else {
            continue;
        };
        let echo_of = (0..results.len()).find(|&sys| {
            results[sys].source_kind == AudioSegmentSourceKind::SystemAudio
                && spans[sys].is_some_and(|(start, end)| {
                    start - slack <= mic_end && mic_start - slack <= end
                })
                && jaccard(&words[mic], &words[sys]) >= ECHO_TEXT_SIMILARITY
        });
        if let Some(sys) = echo_of {
            echoed[mic] = true;
            results[sys].rank = results[sys].rank.min(results[mic].rank);
        }
    }
    results
        .into_iter()
        .zip(echoed)
        .filter_map(|(result, echoed)| (!echoed).then_some(result))
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// Folds each screen moment into a better one (earlier in `groups`, which is
/// best-first) from the same app within [`SCREEN_MOMENT_WINDOW_SECS`] of its
/// frame. The better moment keeps its frame; times and counts widen to cover
/// both. A moment with no app identity or time never folds.
pub(super) fn merge_same_minute_screen_moments(
    groups: Vec<FrameSearchResult>,
) -> Vec<FrameSearchResult> {
    let mut kept: Vec<(FrameSearchResult, Option<(String, OffsetDateTime)>)> = Vec::new();
    for group in groups {
        let app = group
            .app_bundle_id
            .as_deref()
            .or(group.app_name.as_deref())
            .map(|app| app.trim().to_lowercase())
            .filter(|app| !app.is_empty());
        let key = app.zip(parse_at(&group.representative_frame.captured_at));
        let into = key.as_ref().and_then(|(app, at)| {
            kept.iter_mut().find(|(_, other)| {
                other.as_ref().is_some_and(|(other_app, other_at)| {
                    other_app == app
                        && (*other_at - *at).whole_seconds().abs() < SCREEN_MOMENT_WINDOW_SECS
                })
            })
        });
        match into {
            Some((into, _)) => absorb_screen_moment(into, group),
            None => kept.push((group, key)),
        }
    }
    kept.into_iter().map(|(group, _)| group).collect()
}

fn absorb_screen_moment(into: &mut FrameSearchResult, other: FrameSearchResult) {
    if other.group_start_at < into.group_start_at {
        into.group_start_at = other.group_start_at;
    }
    if other.group_end_at > into.group_end_at {
        into.group_end_at = other.group_end_at;
    }
    into.match_count += other.match_count;
    into.secret_redaction_count = into
        .secret_redaction_count
        .max(other.secret_redaction_count);
    into.has_secret_redactions |= other.has_secret_redactions;
    // A keyword match anywhere in the moment beats a meaning-only excerpt.
    if into.found_by_meaning && !other.found_by_meaning {
        into.snippet = other.snippet;
        into.found_by_meaning = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{processing::Frame, AudioSegment, FrameEquivalence};

    fn audio(
        id: i64,
        kind: AudioSegmentSourceKind,
        at: &str,
        snippet: &str,
        rank: f64,
    ) -> AudioSearchResult {
        let segment = AudioSegment {
            id,
            source_kind: kind.clone(),
            source_session_id: format!("session-{id}"),
            segment_index: 0,
            file_path: format!("/tmp/{id}.m4a"),
            started_at: at.to_string(),
            ended_at: at.to_string(),
            capture_segment_id: None,
            created_at: at.to_string(),
            updated_at: at.to_string(),
        };
        AudioSearchResult {
            rank,
            group_key: format!("audio:{id}"),
            audio_segment: segment,
            source_kind: kind,
            span_start_ms: 0,
            span_end_ms: 3_000,
            absolute_start_at: at.to_string(),
            absolute_end_at: at.to_string(),
            match_count: 1,
            snippet: snippet.to_string(),
            aligned_frame: None,
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        }
    }

    fn screen(id: i64, app: Option<&str>, at: &str, rank: f64) -> FrameSearchResult {
        FrameSearchResult {
            rank,
            group_key: format!("frame:{id}"),
            representative_frame: Frame {
                id,
                session_id: "screen".to_string(),
                file_path: format!("/tmp/{id}.jpg"),
                captured_at: at.to_string(),
                width: None,
                height: None,
                equivalence: FrameEquivalence {
                    hint: None,
                    proof: None,
                    version: None,
                    status: None,
                    error: None,
                },
                metadata_snapshot: None,
                created_at: at.to_string(),
                updated_at: at.to_string(),
            },
            group_start_at: at.to_string(),
            group_end_at: at.to_string(),
            match_count: 1,
            snippet: format!("hit {id}"),
            app_bundle_id: app.map(str::to_string),
            app_name: None,
            window_title: None,
            browser_url: None,
            thumbnail_frame_id: id,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        }
    }

    #[test]
    fn short_audio_fragment_no_longer_outranks_a_close_screen_match() {
        use AudioSegmentSourceKind::Microphone;
        // BM25 gave the 3-word mic fragment the better raw rank.
        let short = audio(
            1,
            Microphone,
            "2026-05-17T10:00:00Z",
            "EMS <mark>KVM</mark> <mark>switch</mark>,",
            -9.0,
        );
        let long = audio(
            2,
            Microphone,
            "2026-05-17T11:00:00Z",
            "so I ordered the <mark>KVM</mark> <mark>switch</mark> from that store yesterday",
            -7.0,
        );
        let ranked = rank_audio_moments(vec![short, long]);
        // Within audio the full sentence now leads the fragment...
        assert_eq!(ranked[0].audio_segment.id, 2);
        // ...and a screen match at -6.0 beats both (close scores go to screen).
        assert!(
            ranked.iter().all(|r| r.rank > -6.0),
            "{:?}",
            ranked.iter().map(|r| r.rank).collect::<Vec<_>>()
        );
        // A truncated snippet is a long fragment: only the margin applies.
        let truncated = rank_audio_moments(vec![audio(
            3,
            Microphone,
            "2026-05-17T10:00:00Z",
            "...<mark>KVM</mark> on...",
            -10.0,
        )]);
        assert_eq!(truncated[0].rank, -10.0 * AUDIO_VS_SCREEN_MARGIN);
    }

    #[test]
    fn mic_echo_of_system_audio_keeps_only_the_system_copy() {
        use AudioSegmentSourceKind::{Microphone, SystemAudio};
        let text = "we should ship the <mark>KVM</mark> <mark>switch</mark> review on friday";
        let mic = audio(1, Microphone, "2026-05-17T10:00:01Z", text, -9.0);
        let sys = audio(
            2,
            SystemAudio,
            "2026-05-17T10:00:00Z",
            "we should ship the <mark>KVM</mark> <mark>switch</mark> review friday",
            -5.0,
        );
        // Same words, but a minute later: not an echo.
        let later_mic = audio(3, Microphone, "2026-05-17T10:01:00Z", text, -4.0);
        // Overlapping, different words: not an echo.
        let other_mic = audio(
            4,
            Microphone,
            "2026-05-17T10:00:00Z",
            "my own <mark>KVM</mark> <mark>switch</mark> broke last week sadly",
            -3.0,
        );

        let ranked = rank_audio_moments(vec![mic, sys, later_mic, other_mic]);
        let ids: Vec<i64> = ranked.iter().map(|r| r.audio_segment.id).collect();
        assert_eq!(
            ids,
            vec![2, 3, 4],
            "the mic echo drops; system audio takes its rank"
        );
        assert_eq!(ranked[0].source_kind, SystemAudio);
    }

    #[test]
    fn same_app_screen_moments_within_a_minute_fold_into_the_best() {
        let merged = merge_same_minute_screen_moments(vec![
            screen(1, Some("com.amazon"), "2026-05-17T10:00:30Z", -9.0),
            screen(2, Some("com.amazon"), "2026-05-17T10:00:00Z", -8.0),
            screen(3, Some("com.apple.Safari"), "2026-05-17T10:00:10Z", -7.0),
            screen(4, Some("com.amazon"), "2026-05-17T10:01:40Z", -6.0),
            screen(5, None, "2026-05-17T10:00:31Z", -5.0),
        ]);
        let ids: Vec<i64> = merged.iter().map(|g| g.representative_frame.id).collect();
        assert_eq!(ids, vec![1, 3, 4, 5]);
        assert_eq!(merged[0].match_count, 2);
        assert_eq!(merged[0].group_start_at, "2026-05-17T10:00:00Z");
        assert_eq!(merged[0].group_end_at, "2026-05-17T10:00:30Z");
    }
}
