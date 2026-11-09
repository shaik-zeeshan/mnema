//! Ports of `natural-tokens.test.ts` and `question-detect.test.ts`.

use time::macros::datetime;

use super::*;

// Friday 2026-10-09, mid-afternoon.
const NOW: OffsetDateTime = datetime!(2026-10-09 15:00 +05:30);

fn apps() -> Vec<String> {
    ["Arc", "Zed", "Google Chrome", "Notes"]
        .map(String::from)
        .to_vec()
}

fn ops(text: &str) -> String {
    let p = parse_natural(text, NOW, &apps());
    let pills: Vec<Pill> = p.spans.iter().map(|s| s.pill.clone()).collect();
    operator_query(&p.residual, &merge_pills(&[], &pills))
}

#[test]
fn sentences_become_operator_queries() {
    assert_eq!(
        ops("pricing yesterday afternoon in Arc"),
        "pricing app:Arc date:yesterday hours:12-18"
    );
    assert_eq!(ops("invoice in Arc"), "invoice app:Arc");
    assert_eq!(ops("pricing notes"), "pricing notes");
    assert_eq!(ops("from google chrome"), "app:\"Google Chrome\"");
    assert_eq!(ops("in Arcade"), "in Arcade");
}

#[test]
fn weekdays_named_days_and_parts() {
    assert_eq!(ops("deck last Tuesday"), "deck date:2026-10-06");
    assert_eq!(ops("on friday"), "date:2026-10-02");
    assert_eq!(ops("thursday"), "date:yesterday");
    assert_eq!(ops("tuesday morning"), "date:2026-10-06 hours:5-12");
    assert_eq!(ops("today"), "date:today");
    assert_eq!(ops("this week"), "date:this-week");
    assert_eq!(ops("3 days ago"), "date:2026-10-06");
    assert_eq!(ops("this afternoon"), "date:today hours:12-18");
    assert_eq!(ops("tonight"), "date:today hours:18-24");
    assert_eq!(
        ops("standup in the afternoon"),
        "standup date:today hours:12-18"
    );
    assert_eq!(
        ops("afternoon standup yesterday"),
        "standup date:yesterday hours:12-18"
    );
    assert_eq!(ops("evening on tuesday"), "date:2026-10-06 hours:18-24");
    assert_eq!(ops("standup hours:12-18"), "standup hours:12-18");
}

#[test]
fn sources_and_typed_operators() {
    assert_eq!(ops("what Ana said on a call"), "what Ana said source:mic");
    assert_eq!(ops("on my screen"), "source:screen");
    assert_eq!(ops("in a podcast"), "source:system");
    assert_eq!(ops("app:ar"), "app:Arc");
    assert_eq!(ops("source:microphone"), "source:mic");
    let bad = parse_natural("pricing date:tuesdy", NOW, &apps());
    assert!(bad.spans.is_empty());
    assert_eq!(bad.residual, "pricing date:tuesdy");
    assert_eq!(
        parse_natural("what did Ana say yesterday?", NOW, &apps()).residual,
        "what did Ana say?"
    );
}

#[test]
fn merge_narrows_and_orders() {
    let yesterday = Pill::date("yesterday", "Yesterday", None);
    let afternoon = Pill::Date {
        value: None,
        label: String::new(),
        part: Some(Part::Afternoon),
        today: false,
    };
    let narrowed = merge_pills(&[yesterday.clone()], &[afternoon.clone()]);
    assert_eq!(pill_label(&narrowed[0]), "Yesterday · afternoon");
    assert_eq!(
        merge_pills(&[afternoon.clone()], &[yesterday.clone()]),
        narrowed
    );
    let worded = merge_pills(
        &[],
        &[Pill::Date {
            value: None,
            label: String::new(),
            part: Some(Part::Afternoon),
            today: true,
        }],
    );
    assert_eq!(pill_label(&worded[0]), "Today · afternoon");
    let merged = merge_pills(
        &[Pill::Source(Src::Mic), Pill::App("Zed".into())],
        &[Pill::App("Arc".into()), yesterday],
    );
    let ops: Vec<String> = merged.iter().map(pill_operators).collect();
    assert_eq!(ops, ["app:Arc", "date:yesterday", "source:mic"]);
}

#[test]
fn mirror_tints_phrases_and_underlines_the_bad_token() {
    let text = "pricing yesterday date:tuesdy";
    let p = parse_natural(text, NOW, &apps());
    let segs = mirror_segments(text, &p.spans, Some("date:tuesdy"));
    let kinds: Vec<Seg> = segs.iter().map(|s| s.1).collect();
    assert_eq!(kinds, [Seg::Plain, Seg::Live, Seg::Plain, Seg::Bad]);
    assert_eq!(segs[1].0, "yesterday");
}

#[test]
fn ghost_completes_operators_values_and_apps() {
    let a = apps();
    assert_eq!(ghost("so", &a).map(|g| g.shown), Some("urce:".into()));
    assert_eq!(ghost("so", &a).map(|g| g.accepted), Some("source:".into()));
    assert_eq!(
        ghost("date:yes", &a).map(|g| g.accepted),
        Some("date:yesterday ".into())
    );
    assert_eq!(ghost("notes in Ar", &a).map(|g| g.shown), Some("c".into()));
    let quoted = ghost("app:goo", &a).expect("ghost");
    assert_eq!(quoted.shown, "gle Chrome\"");
    assert_eq!(quoted.accepted, "app:\"Google Chrome\" ");
    assert_eq!(ghost("pricing ", &a), None);
}

#[test]
fn questions_scope_and_tab() {
    assert!(is_question("what did Ana say"));
    assert!(is_question("pricing?"));
    assert!(!is_question("what"));
    assert!(!is_question("pricing page"));
    assert!(is_question("summarize"));
    assert_eq!(
        scoped_question("what did I read?", &[Pill::App("Arc".into())]),
        "what did I read in Arc?"
    );
    assert!(same_question("What did Ana say?", "what did  ana say"));
    assert_eq!(next_forced(None, true), Some(Forced::Search));
    assert_eq!(next_forced(Some(Forced::Search), true), None);
    assert_eq!(next_forced(None, false), Some(Forced::Ask));
    assert_eq!(next_forced(Some(Forced::Ask), false), None);
}
