//! A receipt's cited audio (`receipt-audio.ts`, `receipt-audio-loader.ts`,
//! `receipt-lane.ts`): every audio segment in the activity's span, hydrated to
//! its spoken turns (a diarizer that found no words falls back to the
//! segment's transcription), each turn named live from the people directory
//! and coloured in first-appearance order (the account owner pinned to the
//! audio channel's lavender). Pure over the two backends, run off the main
//! thread. Playback lives in `audio_view.rs` on the Timeline's `Transport`.

use crate::insights::data::Backend;
use crate::timeline::audio::data::Backend as AudioBackend;
use crate::timeline::audio::model::{self, Profile, Turn};

/// A span's audio segment (`list_audio_segments`).
#[derive(Clone, Debug, PartialEq)]
pub struct SegRow {
    pub id: i64,
    pub start: i64,
    pub end: i64,
    pub mic: bool,
}

/// `TurnView`: one spoken turn, in wall-clock ms.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnView {
    /// `(audio segment id, turn id)`.
    pub key: (i64, i64),
    pub segment: i64,
    pub seg_start: i64,
    pub start: i64,
    pub end: i64,
    pub speaker: String,
    pub fallback: bool,
    /// 0 = the account owner (`--cat-communication`), 1..=4 the palette.
    pub color: u8,
    /// "microphone" / "system audio".
    pub source: &'static str,
    pub text: String,
    pub cited: bool,
    pub headline: bool,
}

/// The span's turns, and how many segments still wait on transcription.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub turns: Vec<TurnView>,
    /// `(done, total)` segments.
    pub transcribing: (usize, usize),
}

fn clean(label: &str) -> String {
    let l = label.trim();
    let l = l
        .strip_prefix("Maybe ")
        .or_else(|| l.strip_prefix("maybe "))
        .unwrap_or(l);
    l.trim().to_string()
}

fn is_default_label(label: &str) -> bool {
    let c = clean(label).to_lowercase();
    c.strip_prefix("unknown speaker ")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|ch| ch.is_ascii_digit()))
}

/// `isFallbackSpeaker`: the anonymous "Speaker N".
pub fn is_fallback(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n.strip_prefix("speaker ")
        .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
}

/// `speakerDisplay`: the profile's live name, else a cleaned "Speaker N".
pub fn speaker_display(turn: &Turn, profiles: &[Profile]) -> String {
    if let Some(id) = turn.person_id {
        if let Some(p) = profiles.iter().find(|p| p.id == id) {
            if !is_default_label(&p.display_name) {
                return p.display_name.clone();
            }
        }
    }
    let c = clean(&turn.speaker_label);
    match c.get(..8) {
        Some(head) if head.eq_ignore_ascii_case("unknown ") => c[8..].to_string(),
        _ => c,
    }
}

/// `syntheticTurnsFromTranscription`: the transcription's runs as "Voice" turns.
fn synthetic(seg: &SegRow, runs: Vec<model::Run>, text: Option<&str>) -> Vec<Turn> {
    let runs = if runs.is_empty() {
        match text.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => vec![model::Run {
                start_ms: 0,
                end_ms: (seg.end - seg.start).max(0),
                text: t.to_string(),
            }],
            None => return Vec::new(),
        }
    } else {
        runs
    };
    runs.into_iter()
        .enumerate()
        .map(|(i, r)| Turn {
            id: -(i as i64 + 1),
            cluster_id: -1,
            speaker_label: "Voice".into(),
            start_ms: r.start_ms,
            end_ms: r.end_ms,
            transcript_text: Some(r.text),
            ..Default::default()
        })
        .collect()
}

/// The transcription fallback for a turnless segment: `(turns, pending)`.
fn fallback(au: &dyn AudioBackend, seg: &SegRow) -> (Vec<Turn>, bool) {
    let Ok(mut jobs) = au.jobs(seg.id) else {
        return (Vec::new(), false);
    };
    jobs.retain(|j| j.processor == model::TRANSCRIPTION);
    jobs.sort_by(|a, b| b.id.cmp(&a.id));
    let pending = jobs
        .first()
        .is_some_and(|j| j.status == "queued" || j.status == "running");
    let Some(job) = jobs.iter().find(|j| j.status == "completed") else {
        return (Vec::new(), pending);
    };
    match au.result(job.id) {
        Ok(Some(r)) => {
            let (runs, words) = model::transcript(&r);
            let runs = if runs.is_empty() { words } else { runs };
            (synthetic(seg, runs, r.result_text.as_deref()), pending)
        }
        _ => (Vec::new(), pending),
    }
}

/// `loadSpan`: every segment in the span → its turns → the ordered views.
/// `cited` is the activity's audio evidence: `(segment id, headline)`.
pub fn load_span(
    ins: &dyn Backend,
    au: &dyn AudioBackend,
    span: (i64, i64),
    cited: &[(i64, bool)],
) -> Result<Span, String> {
    let profiles = au.profiles().unwrap_or_default();
    let segments = ins.audio_segments(span.0, span.1)?;
    let mut waiting = 0;
    let mut hydrated = Vec::new();
    for seg in &segments {
        let mut turns = au.turns(seg.id).unwrap_or_default();
        let wordless = !turns.iter().any(|t| {
            t.transcript_text
                .as_deref()
                .is_some_and(|x| !x.trim().is_empty())
        });
        if wordless {
            let (fb, pending) = fallback(au, seg);
            if !fb.is_empty() {
                turns = fb;
            }
            if pending {
                waiting += 1;
            }
        }
        hydrated.push((seg.clone(), turns));
    }
    Ok(Span {
        turns: build(&hydrated, cited, &profiles),
        transcribing: (segments.len() - waiting, segments.len()),
    })
}

/// `buildTurnViews`: wordless turns dropped, start-ordered, coloured.
pub fn build(
    segs: &[(SegRow, Vec<Turn>)],
    cited: &[(i64, bool)],
    profiles: &[Profile],
) -> Vec<TurnView> {
    let headline = cited.iter().find(|c| c.1).map(|c| c.0);
    let mut rows: Vec<(bool, TurnView)> = segs
        .iter()
        .flat_map(|(seg, turns)| {
            turns.iter().filter_map(move |t| {
                let text = t
                    .transcript_text
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if text.is_empty() {
                    return None;
                }
                let speaker = speaker_display(t, profiles);
                let owner = t
                    .person_id
                    .and_then(|id| profiles.iter().find(|p| p.id == id))
                    .is_some_and(|p| p.is_account_owner);
                Some((
                    owner,
                    TurnView {
                        key: (seg.id, t.id),
                        segment: seg.id,
                        seg_start: seg.start,
                        start: seg.start + t.start_ms,
                        end: seg.start + t.end_ms,
                        fallback: is_fallback(&speaker),
                        speaker,
                        color: 0,
                        source: if seg.mic {
                            "microphone"
                        } else {
                            "system audio"
                        },
                        text,
                        cited: cited.iter().any(|c| c.0 == seg.id),
                        headline: headline == Some(seg.id),
                    },
                ))
            })
        })
        .collect();
    rows.sort_by(|a, b| a.1.start.cmp(&b.1.start).then(a.1.key.1.cmp(&b.1.key.1)));
    // `assignSpeakerColors`: owners pinned, others cycle four in first-appearance order.
    let mut seen: Vec<(String, u8)> = Vec::new();
    let mut next = 0u8;
    for (owner, r) in &rows {
        if seen.iter().all(|s| s.0 != r.speaker) {
            let c = if *owner {
                0
            } else {
                next += 1;
                (next - 1) % 4 + 1
            };
            seen.push((r.speaker.clone(), c));
        }
    }
    rows.into_iter()
        .map(|(_, mut r)| {
            r.color = seen.iter().find(|s| s.0 == r.speaker).map_or(1, |s| s.1);
            r
        })
        .collect()
}

/// The turn's speaker colour.
pub fn color(c: u8, p: &crate::theme::Palette) -> gpui_kit::Hsla {
    match c {
        0 => p.cat_communication,
        1 => p.cat_meetings,
        2 => p.cat_research,
        3 => p.cat_entertainment,
        _ => p.cat_learning,
    }
}

/// `defaultSelectedKey`: the headline's segment, a cited one, else the first.
pub fn default_key(turns: &[TurnView]) -> Option<(i64, i64)> {
    turns
        .iter()
        .find(|t| t.headline)
        .or_else(|| turns.iter().find(|t| t.cited))
        .or(turns.first())
        .map(|t| t.key)
}

/// `activeKeyAt`: the last turn started by `ms` (the karaoke line).
pub fn active_key_at<'a>(
    turns: impl Iterator<Item = &'a TurnView>,
    ms: Option<i64>,
) -> Option<(i64, i64)> {
    let ms = ms?;
    turns.take_while(|t| t.start <= ms).last().map(|t| t.key)
}

/// `turnAtMs`: the turn whose window holds `ms`.
pub fn turn_at(turns: &[TurnView], ms: i64) -> Option<&TurnView> {
    turns.iter().find(|t| ms >= t.start && ms <= t.end)
}

/// `nextClipTurn`: the first turn of the next distinct segment.
pub fn next_clip(turns: &[TurnView], ended: i64) -> Option<&TurnView> {
    let mut order: Vec<&TurnView> = Vec::new();
    for t in turns {
        if order.iter().all(|o| o.segment != t.segment) {
            order.push(t);
        }
    }
    let i = order.iter().position(|o| o.segment == ended)?;
    order.get(i + 1).copied()
}

/// `turnSpeakerRoster`: distinct names in turn order, unnamed voices nudged.
pub fn roster(turns: &[TurnView]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for t in turns {
        if seen.contains(&t.speaker.as_str()) {
            continue;
        }
        seen.push(&t.speaker);
        out.push(if t.fallback {
            format!("{} (unnamed → name in Timeline)", t.speaker)
        } else {
            t.speaker.clone()
        });
    }
    out.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: i64, person: Option<i64>, label: &str, s: i64, text: &str) -> Turn {
        Turn {
            id,
            person_id: person,
            speaker_label: label.into(),
            start_ms: s,
            end_ms: s + 1000,
            transcript_text: Some(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn turns_name_live_colour_by_appearance_and_drop_wordless() {
        let profiles = vec![
            Profile {
                id: 1,
                display_name: "Ana".into(),
                is_account_owner: true,
                ..Default::default()
            },
            Profile {
                id: 2,
                display_name: "Unknown speaker 4".into(),
                ..Default::default()
            },
        ];
        let seg = SegRow {
            id: 9,
            start: 10_000,
            end: 20_000,
            mic: true,
        };
        let turns = vec![
            turn(3, Some(2), "Unknown speaker 4", 2000, "later"),
            turn(1, None, "Maybe Unknown speaker 2", 0, "hello"),
            turn(2, Some(1), "x", 1000, "hi"),
            turn(4, None, "Unknown speaker 3", 3000, "  "),
        ];
        let v = build(&[(seg, turns)], &[(9, true)], &profiles);
        let names: Vec<_> = v.iter().map(|t| t.speaker.as_str()).collect();
        assert_eq!(names, ["speaker 2", "Ana", "speaker 4"]);
        assert_eq!(v.iter().map(|t| t.color).collect::<Vec<_>>(), [1, 0, 2]);
        assert_eq!(v[1].start, 11_000);
        assert!(v[0].headline && v[0].cited);
        assert_eq!(default_key(&v), Some((9, 1)));
        assert_eq!(active_key_at(v.iter(), Some(11_500)), Some((9, 2)));
        assert_eq!(turn_at(&v, 12_500).map(|t| t.key), Some((9, 3)));
        assert!(next_clip(&v, 9).is_none());
    }

    #[test]
    fn a_wordless_segment_falls_back_to_its_transcription_text() {
        let seg = SegRow {
            id: 1,
            start: 0,
            end: 4000,
            mic: false,
        };
        let t = synthetic(&seg, Vec::new(), Some(" all good "));
        assert_eq!(
            (t.len(), t[0].end_ms, t[0].speaker_label.as_str()),
            (1, 4000, "Voice")
        );
        assert!(synthetic(&seg, Vec::new(), None).is_empty());
    }
}
