//! The drawer's pure view model (ports of `audio-drawer-view.test.ts` cases that
//! carry a rule or a boundary).

use std::collections::HashSet;

use super::*;
use crate::timeline::audio::model::{Cluster, Profile, Run, Src, Turn};

fn turn(id: i64, cluster: i64, start: i64, end: i64, text: &str) -> Turn {
    Turn {
        id,
        cluster_id: cluster,
        speaker_label: format!("Unknown speaker {cluster}"),
        start_ms: start,
        end_ms: end,
        transcript_text: Some(text.into()),
        ..Default::default()
    }
}

fn run(start: i64, end: i64, text: &str) -> Run {
    Run {
        start_ms: start,
        end_ms: end,
        text: text.into(),
    }
}

#[test]
fn consecutive_turns_of_one_cluster_merge_and_wordless_turns_drop() {
    let turns = [
        turn(1, 7, 0, 1000, "hello"),
        turn(2, 7, 1000, 2000, "there"),
        turn(3, 8, 2000, 2500, "  "),
        turn(4, 8, 2500, 3000, "hi"),
    ];
    let g = speaker_groups(&turns, &[]);
    assert_eq!(g.len(), 2);
    assert_eq!((g[0].text.as_str(), g[0].end_ms), ("hello there", 2000));
    assert_eq!(g[0].turn_ids, vec![1, 2]);
    assert_eq!(g[1].turn_ids, vec![4]);
}

#[test]
fn labels_names_and_the_unnamed_rule() {
    assert_eq!(clean_label("Maybe Daniel"), "Daniel");
    assert!(is_default_label("Unknown speaker 3"));
    assert!(is_default_label("Maybe Unknown speaker 12"));
    assert!(!is_default_label("Unknown speaker"));
    assert!(!is_default_label("Daniel"));
    let profiles = [Profile {
        id: 5,
        display_name: "Ana Ruiz".into(),
        ..Default::default()
    }];
    let g = Group {
        cluster_id: 1,
        speaker_label: "Unknown speaker 1".into(),
        person_id: Some(5),
        ..Default::default()
    };
    assert_eq!(persisted_name(&g, &profiles), "Ana Ruiz");
    assert!(!is_unnamed(&g));
    let auto = Cluster {
        id: 1,
        person_id: Some(5),
        person_link_auto: true,
        ..Default::default()
    };
    assert_eq!(cluster_option_label(&auto, &profiles), "Ana Ruiz (auto)");
}

#[test]
fn marks_cycle_colour_and_shape_and_pin_the_owner() {
    let owners: HashSet<i64> = [9].into();
    let m = assign_marks([3, 9, 4, 3, 5, 6, 7], &owners);
    assert_eq!(
        m[&3],
        Mark {
            hue: Hue::Meetings,
            shape: Shape::Circle
        }
    );
    assert_eq!(
        m[&9],
        Mark {
            hue: Hue::Owner,
            shape: Shape::Square
        }
    );
    assert_eq!(m[&4].hue, Hue::Research);
    // The owner takes no colour slot (7 is the 5th other voice: meetings
    // again); the shape cycle counts every voice (7 is the 6th: square).
    assert_eq!(
        m[&7],
        Mark {
            hue: Hue::Meetings,
            shape: Shape::Square
        }
    );
}

#[test]
fn karaoke_tolerance_holds_150ms_after_a_word_and_not_151() {
    let words = [run(1000, 1400, "one"), run(3000, 3400, "two")];
    assert_eq!(active_word(&words, 849), None, "before the 150 ms lead-in");
    assert_eq!(active_word(&words, 850), Some(0));
    assert_eq!(active_word(&words, 1550), Some(0), "150 ms after the end");
    assert_eq!(
        active_word(&words, 1551),
        None,
        "151 ms: the silence clears it"
    );
    assert_eq!(
        active_word(&words, 2850),
        Some(1),
        "the next word's lead-in"
    );
}

#[test]
fn karaoke_never_runs_ahead_of_abutting_words() {
    let words = [run(0, 500, "a"), run(500, 900, "b")];
    assert_eq!(active_word(&words, 499), Some(0));
    assert_eq!(active_word(&words, 500), Some(1));
}

#[test]
fn karaoke_degrades_below_80_percent_coverage_and_on_a_leak() {
    let g = Group {
        start_ms: 0,
        end_ms: 2000,
        text: "alpha beta gamma delta".into(),
        ..Default::default()
    };
    let full = [
        run(0, 400, "alpha"),
        run(400, 800, "beta"),
        run(800, 1200, "gamma"),
        run(1200, 1600, "delta"),
    ];
    assert_eq!(karaoke(&full, &g).map(|w| w.len()), Some(4));
    assert!(karaoke(&full[..2], &g).is_none(), "half the text");
    let leak = [&full[..], &[run(1600, 1900, "somebody else talking here")]].concat();
    assert!(
        karaoke(&leak, &g).is_none(),
        "words the paragraph doesn't own"
    );
    // A word centred on the shared edge belongs to the earlier paragraph only.
    assert!(words_in_range(&[run(1900, 2100, "edge")], 2000, 4000).is_empty());
}

#[test]
fn the_paragraph_highlight_is_start_bounded_and_waits_for_playback() {
    let g = |s| Group {
        start_ms: s,
        ..Default::default()
    };
    let groups = [g(0), g(5000)];
    assert_eq!(active_group(&groups, 6000, false), None);
    assert_eq!(active_group(&groups, 4999, true), Some(0));
    assert_eq!(active_group(&groups, 5000, true), Some(1));
    assert_eq!(active_group(&groups, 99_000, true), Some(1));
}

#[test]
fn a_degraded_click_snaps_to_the_nearest_run_inside_the_turn() {
    let g = Group {
        start_ms: 1000,
        end_ms: 4000,
        ..Default::default()
    };
    let runs = [run(0, 900, "x"), run(800, 2000, "y"), run(3000, 5000, "z")];
    assert_eq!(segment_seek(&runs, &g), 1000, "800 clamped into the turn");
    assert_eq!(segment_seek(&[], &g), 1000);
}

#[test]
fn wave_bars_take_the_speaker_and_floor_at_4_percent() {
    let turns = [turn(1, 7, 0, 500, "a")];
    let bars = wave_bars(&[0.0, 0.5], 2000., &turns);
    assert_eq!(
        bars[0],
        Bar {
            height_pct: 4.,
            cluster_id: Some(7),
            at_ms: 500
        }
    );
    assert_eq!(bars[1].cluster_id, None);
    assert_eq!(bars[1].height_pct, 50.);
    assert!(wave_bars(&[], 2000., &turns).is_empty());
}

#[test]
fn the_name_field_rejects_empty_and_placeholder_names() {
    assert!(name_warning("  ").is_some());
    assert!(name_warning("Unknown speaker 4").is_some());
    assert!(name_warning("speaker").is_some());
    assert!(name_warning("Voice 2").is_some());
    assert!(name_warning("Ana").is_none());
}

#[test]
fn status_and_panel_cover_every_branch() {
    use TranscriptStatus as S;
    assert_eq!(
        status_pill(Src::Sys, S::Running, false, false, 0).1,
        "detecting speech"
    );
    assert_eq!(
        status_pill(Src::Mic, S::Success, false, false, 2).1,
        "transcribed · 2 speakers"
    );
    assert_eq!(
        status_pill(Src::Mic, S::Success, true, false, 2).1,
        "finding speakers"
    );
    assert_eq!(panel(S::Success, true, false, 3), Panel::SpeakersFailed);
    assert_eq!(panel(S::Success, true, true, 3), Panel::Reader);
    assert_eq!(panel(S::Success, false, false, 0), Panel::NoSpeech);
    assert_eq!(panel(S::Missing, false, false, 0), Panel::NotRun);
    assert_eq!(stage_label(Src::Sys, "audio_transcription"), "stage 2 of 3");
    assert_eq!(compact_duration(108_000), "1m 48s");
    assert_eq!(player_time(65.9), "1:05");
}
